use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{bail, Context};
use parking_lot::Mutex;
use yrs::{updates::decoder::Decode, Doc, GetString, ReadTxn, StateVector, Text, Transact, Update};

use crate::{links, vault};

const STATE_DIR: &str = ".lownotes/crdt";

#[derive(Clone, Default)]
pub struct CrdtManager {
    docs: Arc<Mutex<HashMap<PathBuf, Doc>>>,
}

pub struct AppliedUpdate {
    pub state: Vec<u8>,
    pub changed: bool,
    pub conflict_path: Option<String>,
}

impl CrdtManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state_relative_path(path: &str) -> String {
        format!("{STATE_DIR}/{}.bin", blake3::hash(path.as_bytes()).to_hex())
    }

    fn state_file(vault_path: &Path, path: &str) -> PathBuf {
        vault_path.join(Self::state_relative_path(path))
    }

    pub(crate) fn encode_state(doc: &Doc) -> Vec<u8> {
        doc.transact().encode_diff_v1(&StateVector::default())
    }

    pub(crate) fn decode_file<'a>(relative: &str, bytes: &'a [u8]) -> anyhow::Result<(String, &'a [u8])> {
        if bytes.len() < 4 {
            bail!("invalid CRDT state header");
        }
        let len = u32::from_be_bytes(bytes[..4].try_into()?) as usize;
        if len == 0 || len > 4096 || bytes.len() < 4 + len {
            bail!("invalid CRDT state path length");
        }
        let path = std::str::from_utf8(&bytes[4..4 + len])?.to_string();
        if !vault::is_markdown(Path::new(&path)) || Self::state_relative_path(&path) != relative {
            bail!("invalid CRDT state path");
        }
        Ok((path, &bytes[4 + len..]))
    }

    pub(crate) fn write_state(vault_path: &Path, path: &str, state: &[u8]) -> anyhow::Result<()> {
        let file = Self::state_file(vault_path, path);
        fs::create_dir_all(file.parent().context("CRDT state directory")?)?;
        let path_bytes = path.as_bytes();
        let mut bytes = Vec::with_capacity(4 + path_bytes.len() + state.len());
        bytes.extend_from_slice(&(path_bytes.len() as u32).to_be_bytes());
        bytes.extend_from_slice(path_bytes);
        bytes.extend_from_slice(state);
        let relative = Self::state_relative_path(path);
        crate::storage::write_validated(&file, &bytes, |data| {
            Self::decode_file(&relative, data).is_ok_and(|(_, update)| Update::decode_v1(update).is_ok())
        })?;
        Ok(())
    }

    fn load_doc(vault_path: &Path, path: &str, initial_text: &str) -> anyhow::Result<Doc> {
        let relative = Self::state_relative_path(path);
        if let Some(bytes) = Self::read_state_file(vault_path, &relative)? {
            let (saved_path, update) = Self::decode_file(&Self::state_relative_path(path), &bytes)?;
            if saved_path != path {
                bail!("CRDT state belongs to another note");
            }
            let doc = Doc::new();
            doc.transact_mut()
                .apply_update(Update::decode_v1(update)?)?;
            return Ok(doc);
        }

        // Matching Markdown files share the same initial Yjs IDs on every peer.
        let mut seed = blake3::Hasher::new();
        seed.update(path.as_bytes());
        seed.update(&[0]);
        seed.update(initial_text.as_bytes());
        let client_id =
            u64::from_le_bytes(seed.finalize().as_bytes()[..8].try_into()?) & ((1u64 << 53) - 1);
        let doc = Doc::with_client_id(client_id);
        if !initial_text.is_empty() {
            let text = doc.get_or_insert_text("content");
            text.push(&mut doc.transact_mut(), initial_text);
        }
        let state = Self::encode_state(&doc);
        Self::write_state(vault_path, path, &state)?;
        // Only the seed uses a stable client ID. Later replacements made by
        // external Markdown editors need a distinct ID on each device.
        let local_doc = Doc::new();
        local_doc
            .transact_mut()
            .apply_update(Update::decode_v1(&state)?)?;
        Ok(local_doc)
    }

    pub fn read_state_file(vault_path: &Path, relative: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let file = vault::safe_join(vault_path, relative)?;
        crate::storage::read_validated(&file, |data| {
            Self::decode_file(relative, data).is_ok_and(|(_, update)| Update::decode_v1(update).is_ok())
        })
    }

    fn ensure_doc<'a>(
        docs: &'a mut HashMap<PathBuf, Doc>,
        vault_path: &Path,
        path: &str,
    ) -> anyhow::Result<&'a Doc> {
        if !vault::is_markdown(Path::new(path)) {
            bail!("CRDT path must be a Markdown note");
        }
        let target = vault::safe_join(vault_path, path)?;
        if crate::note_transaction::recover_note(vault_path, path)? { docs.remove(&target); }
        let state_file = Self::state_file(vault_path, path);
        let file_content = match vault::read_note(vault_path, path) {
            Ok(content) => content,
            Err(_) if !target.exists() => String::new(),
            Err(error) => return Err(error),
        };
        if !docs.contains_key(&target) {
            let doc = Self::load_doc(vault_path, path, &file_content)?;
            docs.insert(target.clone(), doc);
        }
        let doc = docs.get(&target).expect("document inserted");
        let text = doc.get_or_insert_text("content");
        let current = text.get_string(&doc.transact());
        if current != file_content {
            let markdown_is_newer = fs::metadata(&target).and_then(|meta| meta.modified()).ok()
                > fs::metadata(&state_file)
                    .and_then(|meta| meta.modified())
                    .ok();
            if markdown_is_newer {
                // A change made by an external editor becomes a CRDT replacement.
                let mut txn = doc.transact_mut();
                let len = text.len(&txn);
                if len > 0 {
                    text.remove_range(&mut txn, 0, len);
                }
                if !file_content.is_empty() {
                    text.insert(&mut txn, 0, &file_content);
                }
                drop(txn);
                if let Err(error) = crate::note_transaction::commit(vault_path, path, &file_content, &Self::encode_state(doc)) {
                    docs.remove(&target);
                    return Err(error);
                }
            } else {
                // Preserve a valid divergent Markdown version even if its timestamp
                // is older than the collaborative state (external tools can retain dates).
                if target.exists() {
                    let copy = Self::conflict_path(path, &file_content)?;
                    vault::save_note(vault_path, &copy, &file_content)?;
                    crate::catalog_sync::register_generated(vault_path, &copy)?;
                    crate::storage::report_recovery(&vault::safe_join(vault_path, &copy)?, true);
                }
                crate::note_transaction::commit(vault_path, path, &current, &Self::encode_state(doc))?;
                crate::storage::report_recovery(&target, true);
            }
        }
        Ok(docs.get(&target).expect("document retained"))
    }

    pub fn get_or_create_doc(&self, vault_path: &Path, path: &str) -> anyhow::Result<Vec<u8>> {
        let mut docs = self.docs.lock();
        Ok(Self::encode_state(Self::ensure_doc(
            &mut docs, vault_path, path,
        )?))
    }

    pub(crate) fn invalidate_path(&self, root: &Path, path: &str) -> anyhow::Result<()> {
        let target = vault::safe_join(root, path)?;
        self.docs.lock().retain(|note, _| !note.starts_with(&target));
        Ok(())
    }

    /// Explicit native saves retain the existing collaborative history and
    /// participate in the same durable Markdown/CRDT transaction as typing.
    pub fn replace_note_text(&self, root: &Path, path: &str, content: &str) -> anyhow::Result<AppliedUpdate> {
        vault::read_note(root, path)?;
        if content.len() as u64 > vault::MAX_NOTE_BYTES { bail!("errors.noteTooLarge"); }
        let mut docs = self.docs.lock();
        let target = vault::safe_join(root, path)?;
        let current = Self::ensure_doc(&mut docs, root, path)?;
        let before = Self::encode_state(current);
        let candidate = Doc::new();
        candidate.transact_mut().apply_update(Update::decode_v1(&before)?)?;
        let changed = Self::replace_text(&candidate, content);
        if !changed { return Ok(AppliedUpdate { state: before, changed: false, conflict_path: None }); }
        let state = Self::encode_state(&candidate);
        crate::note_transaction::commit(root, path, content, &state)?;
        docs.insert(target, candidate);
        if let Err(error) = links::reconcile_wikilinks(root, path, content) { eprintln!("reconcile_wikilinks failed for {path}: {error}"); }
        Ok(AppliedUpdate { state, changed: true, conflict_path: None })
    }

    pub(crate) fn replace_text(doc: &Doc, content: &str) -> bool {
        let text = doc.get_or_insert_text("content");
        let before = text.get_string(&doc.transact());
        if before == content { return false; }
        let prefix: usize = before.chars().zip(content.chars()).take_while(|(a,b)| a == b).map(|(ch,_)| ch.len_utf8()).sum();
        let suffix: usize = before[prefix..].chars().rev().zip(content[prefix..].chars().rev()).take_while(|(a,b)| a == b).map(|(ch,_)| ch.len_utf8()).sum();
        let mut txn = doc.transact_mut();
        let removed = before.len() - prefix - suffix;
        if removed > 0 { text.remove_range(&mut txn, prefix as u32, removed as u32); }
        let inserted = &content[prefix..content.len()-suffix];
        if !inserted.is_empty() { text.insert(&mut txn, prefix as u32, inserted); }
        true
    }

    pub fn apply_update(
        &self,
        vault_path: &Path,
        path: &str,
        bytes: &[u8],
    ) -> anyhow::Result<AppliedUpdate> {
        self.apply_update_inner(vault_path, path, bytes, false)
    }

    pub fn apply_note_edit(&self, vault_path: &Path, edit: &crate::assistant::NoteEdit) -> anyhow::Result<AppliedUpdate> {
        crate::assistant::validate_edit(edit)?;
        // A proposal may only edit an existing note, never recreate a deleted one.
        vault::read_note(vault_path, &edit.path)?;
        let mut docs = self.docs.lock();
        let target = vault::safe_join(vault_path, &edit.path)?;
        let doc = Self::ensure_doc(&mut docs, vault_path, &edit.path)?;
        let before = Self::encode_state(doc);
        let current = doc.get_or_insert_text("content").get_string(&doc.transact());
        let (from, to, insert) = crate::assistant::edit_range(&current, edit)?;
        // Native Yrs documents use byte offsets. The unchanged edges are trimmed at UTF-8 boundaries.
        let candidate = Doc::new();
        candidate.transact_mut().apply_update(Update::decode_v1(&before)?)?;
        let text = candidate.get_or_insert_text("content");
        let mut txn = candidate.transact_mut();
        if to > from { text.remove_range(&mut txn, from as u32, (to - from) as u32); }
        if !insert.is_empty() { text.insert(&mut txn, from as u32, &insert); }
        drop(txn);
        let state = Self::encode_state(&candidate);
        let content = text.get_string(&candidate.transact());
        crate::note_transaction::commit(vault_path, &edit.path, &content, &state)?;
        docs.insert(target, candidate);
        if let Err(error) = links::reconcile_wikilinks(vault_path, &edit.path, &content) {
            eprintln!("reconcile_wikilinks failed for {}: {error}", edit.path);
        }
        Ok(AppliedUpdate { state, changed: true, conflict_path: None })
    }

    fn apply_update_inner(
        &self,
        vault_path: &Path,
        path: &str,
        bytes: &[u8],
        detect_offline_conflict: bool,
    ) -> anyhow::Result<AppliedUpdate> {
        let update = Update::decode_v1(bytes)?;
        let mut docs = self.docs.lock();
        let target = vault::safe_join(vault_path, path)?;
        let doc = Self::ensure_doc(&mut docs, vault_path, path)?;
        let before = Self::encode_state(doc);
        // Validate and merge in a candidate, leaving cached state unchanged on failure.
        let mut candidate = Doc::new();
        candidate.transact_mut().apply_update(Update::decode_v1(&before)?)?;
        let doc = &candidate;
        let mut resolution = None;
        if detect_offline_conflict {
            let remote = Doc::new();
            remote.transact_mut().apply_update(Update::decode_v1(bytes)?)?;
            let local_content = doc.get_or_insert_text("content");
            let remote_content = remote.get_or_insert_text("content");
            let local_txn = doc.transact();
            let remote_txn = remote.transact();
            let local_vector = local_txn.state_vector();
            let remote_vector = remote_txn.state_vector();
            let local_has_unique = local_vector.iter().any(|(id, clock)| *clock > remote_vector.get(id));
            let remote_has_unique = remote_vector.iter().any(|(id, clock)| *clock > local_vector.get(id));
            let local_text = local_content.get_string(&local_txn);
            let remote_text = remote_content.get_string(&remote_txn);
            if local_text != remote_text && local_has_unique && remote_has_unique {
                // Neither history contains the other. Keep the complete losing
                // version as a note instead of silently interleaving its words.
                let local_wins = blake3::hash(local_text.as_bytes()).as_bytes()
                    >= blake3::hash(remote_text.as_bytes()).as_bytes();
                let (winner, loser) = if local_wins {
                    (local_text, remote_text)
                } else {
                    (remote_text, local_text)
                };
                let local_hash = blake3::hash(&before);
                let remote_hash = blake3::hash(bytes);
                let (left, right) = if local_hash.as_bytes() <= remote_hash.as_bytes() {
                    (local_hash, remote_hash)
                } else {
                    (remote_hash, local_hash)
                };
                let mut seed = blake3::Hasher::new();
                seed.update(path.as_bytes());
                seed.update(left.as_bytes());
                seed.update(right.as_bytes());
                let resolver_id = u64::from_le_bytes(seed.finalize().as_bytes()[..8].try_into()?)
                    & ((1u64 << 53) - 1);
                resolution = Some((winner, loser, resolver_id));
            }
        }
        doc.transact_mut().apply_update(update)?;
        let mut conflict_path = None;
        if let Some((winner, loser, resolver_id)) = resolution {
            let copy_path = Self::conflict_path(path, &loser)?;
            let copy_file = vault::safe_join(vault_path, &copy_path)?;
            if copy_file.exists() {
                if vault::read_note(vault_path, &copy_path)? != loser {
                    bail!("conflict copy already exists with different content");
                }
            } else {
                vault::save_note(vault_path, &copy_path, &loser)?;
                crate::catalog_sync::register_generated(vault_path, &copy_path)?;
                conflict_path = Some(copy_path);
            }
            // Both peers derive the same client ID from the two input states.
            // Concurrent syncs therefore produce the same replacement blocks
            // instead of inserting the winning text twice on reconciliation.
            let resolved = Doc::with_client_id(resolver_id);
            resolved.transact_mut().apply_update(Update::decode_v1(&Self::encode_state(doc))?)?;
            let text = resolved.get_or_insert_text("content");
            let mut txn = resolved.transact_mut();
            let len = text.len(&txn);
            if len > 0 {
                text.remove_range(&mut txn, 0, len);
            }
            if !winner.is_empty() {
                text.insert(&mut txn, 0, &winner);
            }
            drop(txn);
            candidate = resolved;
        }
        let doc = &candidate;
        let state = Self::encode_state(doc);
        let changed = state != before;
        if changed {
            let text = doc
                .get_or_insert_text("content")
                .get_string(&doc.transact());
            if let Err(error) = crate::note_transaction::commit(vault_path, path, &text, &state) {
                docs.remove(&target);
                return Err(error);
            }
            if let Err(error) = links::reconcile_wikilinks(vault_path, path, &text) {
                eprintln!("reconcile_wikilinks failed for {path}: {error}");
            }
            docs.insert(target, candidate);
        }
        Ok(AppliedUpdate { state, changed, conflict_path })
    }

    pub(crate) fn conflict_path(path: &str, content: &str) -> anyhow::Result<String> {
        let note = Path::new(path);
        let stem = note.file_stem().and_then(|s| s.to_str()).context("invalid note name")?;
        let extension = note.extension().and_then(|s| s.to_str()).context("invalid note extension")?;
        let hash = blake3::hash(content.as_bytes()).to_hex();
        let name = format!("{} (conflict {}).{}", stem.chars().take(170).collect::<String>(), &hash[..16], extension);
        Ok(note.with_file_name(name).to_string_lossy().replace('\\', "/"))
    }

    pub fn merge_state_file(
        &self,
        vault_path: &Path,
        relative: &str,
        bytes: &[u8],
    ) -> anyhow::Result<(String, AppliedUpdate)> {
        let (path, update) = Self::decode_file(relative, bytes)?;
        vault::safe_join(vault_path, &path)?;
        let result = self.apply_update_inner(vault_path, &path, update, true)?;
        Ok((path, result))
    }

    pub fn remove_doc(&self, vault_path: &Path, path: &str) -> anyhow::Result<()> {
        let target = vault::safe_join(vault_path, path)?;
        self.docs
            .lock()
            .retain(|note, _| !note.starts_with(&target));
        let state_dir = vault_path.join(STATE_DIR);
        if state_dir.is_dir() {
            for entry in fs::read_dir(state_dir)? {
                let entry = entry?;
                if !entry.file_type()?.is_file() {
                    continue;
                }
                let bytes = fs::read(entry.path())?;
                if let Some(saved_path) = bytes
                    .get(..4)
                    .and_then(|header| <[u8; 4]>::try_from(header).ok())
                    .and_then(|header| bytes.get(4..4 + u32::from_be_bytes(header) as usize))
                    .and_then(|path| std::str::from_utf8(path).ok())
                {
                    if saved_path == path || saved_path.starts_with(&format!("{path}/")) {
                        fs::remove_file(entry.path())?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn divergent_markdown_with_an_older_timestamp_is_preserved_for_review() {
        let root = tempfile::tempdir().unwrap();
        vault::save_note(root.path(), "note.md", "collaborative text").unwrap();
        CrdtManager::new().get_or_create_doc(root.path(), "note.md").unwrap();
        fs::write(root.path().join("note.md"), "external text with retained timestamp").unwrap();
        filetime::set_file_mtime(root.path().join("note.md"), filetime::FileTime::from_unix_time(1, 0)).unwrap();
        CrdtManager::new().get_or_create_doc(root.path(), "note.md").unwrap();
        assert_eq!(vault::read_note(root.path(), "note.md").unwrap(), "collaborative text");
        let copy = CrdtManager::conflict_path("note.md", "external text with retained timestamp").unwrap();
        assert_eq!(vault::read_note(root.path(), &copy).unwrap(), "external text with retained timestamp");
    }

    #[test]
    fn assistant_task_edit_persists_and_merges_with_concurrent_unicode_text() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let original = "# 🙂 Plano\n\n- [ ] Prática **diária**\n\n![foto](lownotes-image:existing.webp)\n";
        vault::save_note(a.path(), "Plan.md", original).unwrap();
        vault::save_note(b.path(), "Plan.md", original).unwrap();
        let manager_a = CrdtManager::new();
        let manager_b = CrdtManager::new();
        let initial = manager_a.get_or_create_doc(a.path(), "Plan.md").unwrap();
        let remote = Doc::with_client_id(123456);
        remote.transact_mut().apply_update(Update::decode_v1(&initial).unwrap()).unwrap();
        remote.get_or_insert_text("content").insert(&mut remote.transact_mut(), 0, "Texto remoto 🙂\n");
        let edit = crate::assistant::NoteEdit { path: "Plan.md".into(), old_text: original.into(), new_text: original.replace("[ ]", "[x]") };
        let applied = manager_a.apply_note_edit(a.path(), &edit).unwrap();
        assert_eq!(vault::read_note(a.path(), "Plan.md").unwrap(), original.replace("[ ]", "[x]"));
        let remote_state = CrdtManager::encode_state(&remote);
        manager_b.apply_update(b.path(), "Plan.md", &remote_state).unwrap();
        manager_b.apply_update(b.path(), "Plan.md", &applied.state).unwrap();
        manager_a.apply_update(a.path(), "Plan.md", &remote_state).unwrap();
        let expected = format!("Texto remoto 🙂\n{}", original.replace("[ ]", "[x]"));
        assert_eq!(vault::read_note(a.path(), "Plan.md").unwrap(), expected);
        assert_eq!(vault::read_note(b.path(), "Plan.md").unwrap(), expected);
        assert_eq!(CrdtManager::new().get_or_create_doc(a.path(), "Plan.md").unwrap(), manager_a.get_or_create_doc(a.path(), "Plan.md").unwrap());
        let before_retry = vault::read_note(a.path(), "Plan.md").unwrap();
        assert!(manager_a.apply_note_edit(a.path(), &edit).is_err());
        assert_eq!(vault::read_note(a.path(), "Plan.md").unwrap(), before_retry);
        vault::save_note(a.path(), "Duplicates.md", "- [ ] Same\n- [ ] Same\n").unwrap();
        let ambiguous = crate::assistant::NoteEdit { path: "Duplicates.md".into(), old_text: "- [ ] Same".into(), new_text: "- [x] Same".into() };
        assert_eq!(manager_a.apply_note_edit(a.path(), &ambiguous).err().unwrap().to_string(), "ai.editAmbiguous");
        let missing = crate::assistant::NoteEdit { path: "Missing.md".into(), ..ambiguous };
        assert!(manager_a.apply_note_edit(a.path(), &missing).is_err());
        assert!(!a.path().join("Missing.md").exists());
    }

    fn temp_vault(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("lownotes-crdt-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn same_markdown_has_same_genesis_and_concurrent_edits_converge() {
        let a = temp_vault("a");
        let b = temp_vault("b");
        vault::save_note(&a, "shared.md", "Hello ").unwrap();
        vault::save_note(&b, "shared.md", "Hello ").unwrap();
        let manager_a = CrdtManager::new();
        let manager_b = CrdtManager::new();
        let initial_a = manager_a.get_or_create_doc(&a, "shared.md").unwrap();
        let initial_b = manager_b.get_or_create_doc(&b, "shared.md").unwrap();
        assert_eq!(initial_a, initial_b);

        let editor_a = Doc::with_client_id(1001);
        editor_a
            .transact_mut()
            .apply_update(Update::decode_v1(&initial_a).unwrap())
            .unwrap();
        let editor_b = Doc::with_client_id(1002);
        editor_b
            .transact_mut()
            .apply_update(Update::decode_v1(&initial_b).unwrap())
            .unwrap();
        editor_a
            .get_or_insert_text("content")
            .push(&mut editor_a.transact_mut(), "Alice");
        editor_b
            .get_or_insert_text("content")
            .push(&mut editor_b.transact_mut(), "Bob");
        let update_a = CrdtManager::encode_state(&editor_a);
        let update_b = CrdtManager::encode_state(&editor_b);
        manager_a.apply_update(&a, "shared.md", &update_a).unwrap();
        manager_b.apply_update(&b, "shared.md", &update_b).unwrap();
        manager_a.apply_update(&a, "shared.md", &update_b).unwrap();
        manager_b.apply_update(&b, "shared.md", &update_a).unwrap();
        let text_a = vault::read_note(&a, "shared.md").unwrap();
        let text_b = vault::read_note(&b, "shared.md").unwrap();
        assert_eq!(text_a, text_b);
        assert!(text_a.contains("Alice"));
        assert!(text_a.contains("Bob"));
        let _ = fs::remove_dir_all(a);
        let _ = fs::remove_dir_all(b);
    }

    #[test]
    fn offline_same_line_edits_keep_a_conflict_copy_and_converge() {
        let a = temp_vault("offline-a");
        let b = temp_vault("offline-b");
        let baseline = format!("{}Original line\n", "Unchanged line\n".repeat(246));
        vault::save_note(&a, "shared.md", &baseline).unwrap();
        vault::save_note(&b, "shared.md", &baseline).unwrap();
        let manager_a = CrdtManager::new();
        let manager_b = CrdtManager::new();
        let state_a = manager_a.get_or_create_doc(&a, "shared.md").unwrap();
        let state_b = manager_b.get_or_create_doc(&b, "shared.md").unwrap();
        assert_eq!(state_a, state_b);

        let first = "Seila vei, coisa pra carai";
        let second = "E agora? O que fazer?";
        for (manager, vault_path, state, client_id, replacement) in [
            (&manager_a, &a, &state_a, 1001, first),
            (&manager_b, &b, &state_b, 1002, second),
        ] {
            let editor = Doc::with_client_id(client_id);
            editor.transact_mut().apply_update(Update::decode_v1(state).unwrap()).unwrap();
            let text = editor.get_or_insert_text("content");
            let mut txn = editor.transact_mut();
            let start = (baseline.len() - "Original line\n".len()) as u32;
            text.remove_range(&mut txn, start, "Original line".len() as u32);
            text.insert(&mut txn, start, replacement);
            drop(txn);
            manager.apply_update(vault_path, "shared.md", &CrdtManager::encode_state(&editor)).unwrap();
        }

        let incoming = manager_a.get_or_create_doc(&a, "shared.md").unwrap();
        let merged = manager_b.merge_state_file(&b, &CrdtManager::state_relative_path("shared.md"), &{
            let mut file = Vec::new();
            file.extend_from_slice(&("shared.md".len() as u32).to_be_bytes());
            file.extend_from_slice(b"shared.md");
            file.extend_from_slice(&incoming);
            file
        }).unwrap().1;
        let copy = merged.conflict_path.expect("offline conflict must create a review copy");
        let original = vault::read_note(&b, "shared.md").unwrap();
        let duplicate = vault::read_note(&b, &copy).unwrap();
        assert!(original.ends_with(&format!("{first}\n")) || original.ends_with(&format!("{second}\n")));
        assert!(!original.contains(&format!("{first}{second}")));
        assert_ne!(original, duplicate);
        assert!(duplicate.ends_with(&format!("{first}\n")) || duplicate.ends_with(&format!("{second}\n")));

        manager_a.apply_update(&a, "shared.md", &merged.state).unwrap();
        assert_eq!(vault::read_note(&a, "shared.md").unwrap(), original);
        let again = manager_b.merge_state_file(&b, &CrdtManager::state_relative_path("shared.md"), &{
            let mut file = Vec::new();
            file.extend_from_slice(&("shared.md".len() as u32).to_be_bytes());
            file.extend_from_slice(b"shared.md");
            file.extend_from_slice(&incoming);
            file
        }).unwrap().1;
        assert!(again.conflict_path.is_none());
        let _ = fs::remove_dir_all(a);
        let _ = fs::remove_dir_all(b);
    }

    #[test]
    fn simultaneous_offline_reconciliation_stays_single_version() {
        let a = temp_vault("parallel-a");
        let b = temp_vault("parallel-b");
        for root in [&a, &b] {
            vault::save_note(root, "shared.md", "Original").unwrap();
        }
        let manager_a = CrdtManager::new();
        let manager_b = CrdtManager::new();
        for (manager, root, id, word) in [
            (&manager_a, &a, 401, "First"),
            (&manager_b, &b, 402, "Second"),
        ] {
            let state = manager.get_or_create_doc(root, "shared.md").unwrap();
            let editor = Doc::with_client_id(id);
            editor.transact_mut().apply_update(Update::decode_v1(&state).unwrap()).unwrap();
            let text = editor.get_or_insert_text("content");
            let mut txn = editor.transact_mut();
            text.remove_range(&mut txn, 0, 8);
            text.insert(&mut txn, 0, word);
            drop(txn);
            manager.apply_update(root, "shared.md", &CrdtManager::encode_state(&editor)).unwrap();
        }
        let before_a = manager_a.get_or_create_doc(&a, "shared.md").unwrap();
        let before_b = manager_b.get_or_create_doc(&b, "shared.md").unwrap();
        let wrap = |state: &[u8]| {
            let mut file = Vec::new();
            file.extend_from_slice(&("shared.md".len() as u32).to_be_bytes());
            file.extend_from_slice(b"shared.md");
            file.extend_from_slice(state);
            file
        };
        manager_a.merge_state_file(&a, &CrdtManager::state_relative_path("shared.md"), &wrap(&before_b)).unwrap();
        manager_b.merge_state_file(&b, &CrdtManager::state_relative_path("shared.md"), &wrap(&before_a)).unwrap();
        let after_a = manager_a.get_or_create_doc(&a, "shared.md").unwrap();
        let after_b = manager_b.get_or_create_doc(&b, "shared.md").unwrap();
        manager_a.apply_update(&a, "shared.md", &after_b).unwrap();
        manager_b.apply_update(&b, "shared.md", &after_a).unwrap();
        let final_a = vault::read_note(&a, "shared.md").unwrap();
        let final_b = vault::read_note(&b, "shared.md").unwrap();
        assert_eq!(final_a, final_b);
        assert!(final_a == "First" || final_a == "Second", "{final_a:?}");
        let _ = fs::remove_dir_all(a);
        let _ = fs::remove_dir_all(b);
    }

    #[test]
    fn persisted_state_survives_manager_restart() {
        let vault_path = temp_vault("restart");
        vault::save_note(&vault_path, "note.md", "Start").unwrap();
        let manager = CrdtManager::new();
        let state = manager.get_or_create_doc(&vault_path, "note.md").unwrap();
        let editor = Doc::with_client_id(2001);
        editor
            .transact_mut()
            .apply_update(Update::decode_v1(&state).unwrap())
            .unwrap();
        editor
            .get_or_insert_text("content")
            .push(&mut editor.transact_mut(), " end");
        manager
            .apply_update(&vault_path, "note.md", &CrdtManager::encode_state(&editor))
            .unwrap();
        let restarted = CrdtManager::new();
        assert_eq!(
            restarted.get_or_create_doc(&vault_path, "note.md").unwrap(),
            CrdtManager::encode_state(&editor)
        );
        let _ = fs::remove_dir_all(vault_path);
    }

    #[test]
    fn newer_external_markdown_edit_enters_crdt_history() {
        let vault_path = temp_vault("external");
        vault::save_note(&vault_path, "note.md", "Original").unwrap();
        let manager = CrdtManager::new();
        manager.get_or_create_doc(&vault_path, "note.md").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        vault::save_note(&vault_path, "note.md", "Changed outside the app").unwrap();
        let state = manager.get_or_create_doc(&vault_path, "note.md").unwrap();
        let restored = Doc::new();
        restored
            .transact_mut()
            .apply_update(Update::decode_v1(&state).unwrap())
            .unwrap();
        assert_eq!(
            restored
                .get_or_insert_text("content")
                .get_string(&restored.transact()),
            "Changed outside the app"
        );
        let _ = fs::remove_dir_all(vault_path);
    }

    #[test]
    fn removing_folder_clears_nested_crdt_history() {
        let vault_path = temp_vault("folder");
        vault::save_note(&vault_path, "folder/a.md", "A").unwrap();
        vault::save_note(&vault_path, "folder/nested/b.md", "B").unwrap();
        vault::save_note(&vault_path, "other.md", "Other").unwrap();
        let manager = CrdtManager::new();
        for path in ["folder/a.md", "folder/nested/b.md", "other.md"] {
            manager.get_or_create_doc(&vault_path, path).unwrap();
        }
        manager.remove_doc(&vault_path, "folder").unwrap();
        assert!(!CrdtManager::state_file(&vault_path, "folder/a.md").exists());
        assert!(!CrdtManager::state_file(&vault_path, "folder/nested/b.md").exists());
        assert!(CrdtManager::state_file(&vault_path, "other.md").exists());
        let _ = fs::remove_dir_all(vault_path);
    }
}
