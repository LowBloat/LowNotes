//! A creation is durable before either its catalog identity or its file appears.
//! Recovery follows the identity's current path and never replays a seed over
//! subsequent edits or an unrelated file created at the reserved filename.
use crate::{
    catalog::{self, Catalog, Change, Entry, Location},
    crdt::CrdtManager,
    storage, structural, vault,
};
use anyhow::{bail, Context};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    cell::Cell,
    fs,
    path::{Path, PathBuf},
};
use yrs::{updates::decoder::Decode, Doc, GetString, Map, ReadTxn, Text, Transact, Update};

const DIRECTORY: &str = ".lownotes/pending-create";
const IDENTITY: &str = "lownotes.note-identity.v1";
thread_local! { static BUSY: Cell<bool> = const { Cell::new(false) }; }
struct Scope(bool);
impl Scope {
    fn enter() -> Self {
        Self(BUSY.replace(true))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        BUSY.set(self.0);
    }
}

#[derive(Serialize, Deserialize)]
struct Intent {
    version: u8,
    path: String,
    id: String,
    is_dir: bool,
    catalog: Catalog,
    content: Option<String>,
    state: Option<String>,
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '\\' | ':'))
        && path
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.'))
}

/// Inspect existing ancestors without creating directories before the intent.
/// This also retains the assistant's protection against redirected parents.
fn contained(root: &Path, relative: &str) -> anyhow::Result<PathBuf> {
    let root = root.canonicalize()?;
    let target = vault::safe_join(&root, relative)?;
    let mut current = root.clone();
    for part in target
        .parent()
        .context("missing creation parent")?
        .strip_prefix(&root)?
        .components()
    {
        current.push(part);
        if current.exists() && !current.canonicalize()?.starts_with(&root) {
            bail!("errors.pathEscape");
        }
    }
    Ok(target)
}

fn decode(state: &[u8]) -> anyhow::Result<Doc> {
    let doc = Doc::new();
    doc.transact_mut().apply_update(Update::decode_v1(state)?)?;
    Ok(doc)
}
fn text(doc: &Doc) -> String {
    doc.get_or_insert_text("content")
        .get_string(&doc.transact())
}

fn seed(id: &str, content: &str) -> Vec<u8> {
    let hash = blake3::hash(format!("created:{id}").as_bytes());
    let client = u64::from_le_bytes(hash.as_bytes()[..8].try_into().unwrap()) & ((1u64 << 53) - 1);
    let doc = Doc::with_client_id(client);
    let identity = doc.get_or_insert_map(IDENTITY);
    let content_text = doc.get_or_insert_text("content");
    let mut txn = doc.transact_mut();
    identity.insert(&mut txn, "id", id);
    if !content.is_empty() {
        content_text.push(&mut txn, content);
    }
    drop(txn);
    CrdtManager::encode_state(&doc)
}

fn parse(bytes: &[u8]) -> anyhow::Result<Intent> {
    // Includes a bounded catalog, JSON-escaped Markdown and its base64 state.
    if bytes.len() > 128 * 1024 * 1024 {
        bail!("creation intent is too large");
    }
    let intent: Intent = serde_json::from_slice(bytes)?;
    if intent.version != 1 || !valid_path(&intent.path) {
        bail!("invalid creation intent");
    }
    let resolved = intent.catalog.resolve()?;
    let entry = resolved
        .get(&intent.id)
        .context("missing creation identity")?;
    if entry.deleted() || entry.path != intent.path || entry.is_dir != intent.is_dir {
        bail!("creation intent does not match catalog");
    }
    match (&intent.content, &intent.state, intent.is_dir) {
        (None, None, true) => (),
        (Some(content), Some(state), false) if content.len() as u64 <= vault::MAX_NOTE_BYTES => {
            let state = STANDARD.decode(state)?;
            let doc = decode(&state)?;
            let identity = doc.get_or_insert_map(IDENTITY);
            if text(&doc) != *content
                || identity
                    .get(&doc.transact(), "id")
                    .is_none_or(|value| value.to_string(&doc.transact()) != intent.id)
            {
                bail!("creation state does not match its text or identity");
            }
        }
        _ => bail!("invalid creation payload"),
    }
    Ok(intent)
}

fn owns(root: &Path, path: &str, initial: &[u8]) -> anyhow::Result<bool> {
    let Some(bytes) = CrdtManager::read_state_file(root, &CrdtManager::state_relative_path(path))?
    else {
        return Ok(false);
    };
    let (_, update) = CrdtManager::decode_file(&CrdtManager::state_relative_path(path), &bytes)?;
    let current = decode(update)?;
    let original = decode(initial)?;
    let observed = current.transact().state_vector();
    let original_vector = original.transact().state_vector();
    Ok(original_vector
        .iter()
        .all(|(client, clock)| observed.get(client) >= *clock))
}

fn preserve_original(root: &Path, path: &str, content: &str, state: &[u8]) -> anyhow::Result<()> {
    let copy = CrdtManager::conflict_path(path, content)?;
    let target = vault::safe_join(root, &copy)?;
    if target.exists() {
        if fs::read_to_string(&target)? != content {
            bail!("creation recovery copy already has different content");
        }
    } else {
        crate::note_transaction::commit(root, &copy, content, state)?;
    }
    crate::catalog_sync::register_generated(root, &copy)?;
    storage::report_recovery(&target, true);
    Ok(())
}

fn finish(
    root: &Path,
    file: &Path,
    intent: &Intent,
    manager: &CrdtManager,
    recovered: bool,
    hook: &impl Fn(u8) -> anyhow::Result<()>,
) -> anyhow::Result<String> {
    let mut catalog = catalog::transact(root, |current| {
        *current = current.merged(&intent.catalog)?;
        Ok(current.clone())
    })?;
    hook(1)?;
    let mut entry = catalog
        .resolve()?
        .remove(&intent.id)
        .context("missing recovered creation")?;
    let initial = intent
        .state
        .as_ref()
        .map(|state| STANDARD.decode(state))
        .transpose()?;
    // Catalog changes can arrive after publication but before creation cleanup.
    // Locate the actual seed by its history, bind it, and let the existing
    // structural journal move/delete it before completing this creation.
    let mut paths = structural::load_paths(root)?;
    let candidates = paths
        .paths
        .get(&intent.id)
        .into_iter()
        .cloned()
        .chain(std::iter::once(intent.path.clone()))
        .chain(std::iter::once(entry.path.clone()))
        .chain(entry.aliases.iter().cloned());
    let mut physical = None;
    for path in candidates {
        let target = contained(root, &path)?;
        let owned = match &initial {
            Some(state) => target.is_file() && owns(root, &path, state)?,
            None => target.is_dir(),
        };
        if owned {
            physical = Some(path);
            break;
        }
    }
    let archived_projection = entry.deleted() && physical.is_some();
    if let Some(old) = physical {
        paths.paths.insert(intent.id.clone(), old.clone());
        for (id, parent) in intent.catalog.resolve()? {
            if parent.is_dir
                && intent.path.starts_with(&format!("{}/", parent.path))
                && root.join(&parent.path).is_dir()
            {
                paths.paths.entry(id).or_insert(parent.path);
            }
        }
        structural::save_paths(root, &paths)?;
        if entry.deleted() || old != entry.path {
            crate::catalog_sync::merge(root, manager, &catalog, "recovery")?;
            catalog = catalog::load(root)?;
            entry = catalog
                .resolve()?
                .remove(&intent.id)
                .context("missing materialized creation")?;
        }
    }
    if entry.deleted() {
        if !archived_projection {
            crate::catalog_sync::archive_creation(
                root,
                &entry,
                intent.content.as_deref(),
                initial.as_deref(),
                &catalog,
            )?;
            if let (Some(content), Some(state)) = (&intent.content, &initial) {
                crate::catalog_sync::preserve_deleted(root, &entry, content, Some(state))?;
            }
        }
        storage::remove_file(&storage::backup_path(file))?;
        storage::remove_file(file)?;
        fs::remove_dir(file.parent().context("missing journal directory")?)?;
        storage::report_recovery(&root.join(crate::catalog_sync::TRASH), true);
        return Ok(entry.path);
    }
    let mut target = contained(root, &entry.path)?;
    let already_owned = match &initial {
        Some(state) => owns(root, &entry.path, state)?,
        None => target.is_dir(),
    };
    if !intent.is_dir && target.exists() && !already_owned {
        // Keep the external file at its filename, assign it an independent ID,
        // and place this creation at a stable alternative. Publish both actions
        // together so interruption during this decision is safe to replay.
        catalog = catalog::transact(root, |current| {
            let resolved = current.resolve()?;
            let original = &resolved[&intent.id];
            let mut counter = 0;
            let location = loop {
                let name =
                    catalog::conflict_name(&original.location.name, &intent.id, false, counter);
                let location = Location {
                    parent: original.location.parent.clone(),
                    name,
                };
                let path = original
                    .path
                    .rsplit_once('/')
                    .map(|(parent, _)| format!("{parent}/{}", location.name))
                    .unwrap_or_else(|| location.name.clone());
                if !root.join(&path).exists()
                    && !resolved
                        .values()
                        .any(|entry| !entry.deleted() && entry.path == path)
                {
                    break location;
                }
                counter += 1;
            };
            let old_location = original.location.clone();
            current.push(
                "recovery",
                Change::Move {
                    id: intent.id.clone(),
                    location,
                },
            )?;
            current.push(
                "recovery",
                Change::Create {
                    id: catalog::random_id(),
                    entry: Entry {
                        is_dir: target.is_dir(),
                        location: old_location,
                    },
                },
            )?;
            Ok(current.clone())
        })?;
        entry = catalog.resolve()?.remove(&intent.id).unwrap();
        target = contained(root, &entry.path)?;
        storage::report_recovery(&target, true);
    }
    if intent.is_dir {
        fs::create_dir_all(&target)?;
    } else if let (Some(content), Some(initial)) = (&intent.content, &initial) {
        if already_owned {
            manager.invalidate_path(root, &entry.path)?;
            let saved =
                CrdtManager::read_state_file(root, &CrdtManager::state_relative_path(&entry.path))?
                    .context("missing created state")?;
            let (_, update) =
                CrdtManager::decode_file(&CrdtManager::state_relative_path(&entry.path), &saved)?;
            let saved_text = text(&decode(update)?);
            let external_edit =
                fs::read_to_string(&target).is_ok_and(|markdown| markdown != saved_text);
            let current = manager.get_or_create_doc(root, &entry.path)?;
            // Reconciliation can import an external editor's newer text. Keep
            // the original creation visible as well rather than hiding it only
            // in the retained CRDT history during recovery.
            if recovered
                && external_edit
                && saved_text == *content
                && text(&decode(&current)?) != *content
            {
                preserve_original(root, &entry.path, content, initial)?;
            }
        } else {
            crate::note_transaction::commit_with_hook(root, &entry.path, content, initial, |at| {
                hook(5 + at)
            })?;
            manager.invalidate_path(root, &entry.path)?;
        }
    }
    hook(2)?;
    let mut paths = structural::load_paths(root)?;
    for (id, item) in catalog.resolve()? {
        if !item.deleted() && (id == intent.id || item.is_dir) && root.join(&item.path).exists() {
            paths.paths.insert(id, item.path);
        }
    }
    structural::save_paths(root, &paths)?;
    hook(3)?;
    // Backup first; removing the primary before it could resurrect a completed
    // creation and overwrite a subsequent edit on the next recovery.
    storage::remove_file(&storage::backup_path(file))?;
    storage::remove_file(file)?;
    hook(4)?;
    fs::remove_dir(file.parent().context("missing journal directory")?)?;
    if recovered {
        storage::report_recovery(&target, true);
    }
    Ok(entry.path)
}

pub fn recover_all(root: &Path, manager: &CrdtManager) -> anyhow::Result<()> {
    if BUSY.get() {
        return Ok(());
    }
    structural::exclusive(root, manager, || {
        let _scope = Scope::enter();
        let directory = root.join(DIRECTORY);
        if !directory.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !entry.file_type()?.is_dir()
                || name.len() != 64
                || !name.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                continue;
            }
            let file = entry.path().join("intent.json");
            let Some(bytes) = storage::read_validated(&file, |bytes| parse(bytes).is_ok())? else {
                if fs::read_dir(entry.path())?.next().is_some() {
                    storage::report_recovery(&entry.path(), false);
                    bail!("creation recovery data has no valid intent");
                }
                fs::remove_dir(entry.path())?;
                continue;
            };
            let intent = parse(&bytes)?;
            if let Err(error) = finish(root, &file, &intent, manager, true, &|_| Ok(())) {
                storage::report_recovery(&file, false);
                return Err(error);
            }
        }
        Ok(())
    })
}

pub fn create(
    root: &Path,
    path: &str,
    content: Option<&str>,
    manager: &CrdtManager,
    author: &str,
) -> anyhow::Result<String> {
    create_with_hook(root, path, content, manager, author, |at| {
        #[cfg(test)]
        if std::env::var("LOWNOTES_CREATION_CRASH_PHASE")
            .ok()
            .and_then(|value| value.parse::<u8>().ok())
            == Some(at)
        {
            std::process::exit(86);
        }
        let _ = at;
        Ok(())
    })
}

fn create_with_hook(
    root: &Path,
    path: &str,
    content: Option<&str>,
    manager: &CrdtManager,
    author: &str,
    hook: impl Fn(u8) -> anyhow::Result<()>,
) -> anyhow::Result<String> {
    structural::exclusive(root, manager, || {
        let _scope = Scope::enter();
        if !valid_path(path) || (content.is_some() && !vault::is_markdown(Path::new(path))) {
            bail!("errors.pathEscape");
        }
        if content.is_some_and(|text| text.len() as u64 > vault::MAX_NOTE_BYTES) {
            bail!("errors.noteTooLarge");
        }
        let target = contained(root, path)?;
        if content.is_some() && target.exists() {
            bail!("errors.noteExists");
        }
        if content.is_none() && target.exists() && !target.is_dir() {
            bail!("errors.noteExists");
        }
        let mut planned = crate::catalog_sync::prepare(root, manager, author)?;
        let mut parent = None;
        let mut prefix = String::new();
        let mut id = String::new();
        let parts: Vec<_> = path.split('/').collect();
        for (index, name) in parts.iter().enumerate() {
            if index != 0 {
                prefix.push('/');
            }
            prefix.push_str(name);
            let is_dir = index + 1 < parts.len() || content.is_none();
            if let Some(existing) = planned
                .resolve()?
                .into_values()
                .find(|entry| !entry.deleted() && entry.path == prefix)
            {
                if existing.is_dir != is_dir {
                    bail!("errors.noteExists");
                }
                if !is_dir {
                    bail!("errors.noteExists");
                }
                id = existing.id;
            } else {
                id = catalog::random_id();
                planned.push(
                    author,
                    Change::Create {
                        id: id.clone(),
                        entry: Entry {
                            is_dir,
                            location: Location {
                                parent: parent.clone(),
                                name: (*name).into(),
                            },
                        },
                    },
                )?;
            }
            parent = Some(id.clone());
        }
        if content.is_some() { crate::note_history::forget(root, path)?; }
        let intent = Intent {
            version: 1,
            path: path.into(),
            id: id.clone(),
            is_dir: content.is_none(),
            catalog: planned,
            content: content.map(str::to_owned),
            state: content.map(|content| STANDARD.encode(seed(&id, content))),
        };
        let directory = contained(
            root,
            &format!("{DIRECTORY}/{}/intent.json", catalog::random_id()),
        )?
        .parent()
        .unwrap()
        .to_path_buf();
        fs::create_dir_all(&directory)?;
        let file = directory.join("intent.json");
        storage::write_validated(&file, &serde_json::to_vec(&intent)?, |bytes| {
            parse(bytes).is_ok()
        })?;
        hook(0)?;
        finish(root, &file, &intent, manager, false, &hook)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active_id(root: &Path, path: &str) -> String {
        catalog::load(root)
            .unwrap()
            .resolve()
            .unwrap()
            .into_values()
            .find(|entry| !entry.deleted() && entry.path == path)
            .unwrap()
            .id
    }

    #[test]
    fn interrupted_creation_reuses_no_deleted_identity_and_retains_both_external_files() {
        for phase in [0, 1] {
            let root = tempfile::tempdir().unwrap();
            let manager = CrdtManager::new();
            create(root.path(), "note.md", Some("old"), &manager, "a").unwrap();
            let old_id = active_id(root.path(), "note.md");
            crate::catalog_sync::delete(root.path(), "note.md", &manager, "a").unwrap();
            assert!(create_with_hook(
                root.path(),
                "note.md",
                Some("new 🙂"),
                &manager,
                "a",
                |at| {
                    if at == phase {
                        bail!("interrupted");
                    }
                    Ok(())
                }
            )
            .is_err());
            fs::write(root.path().join("note.md"), "external replacement").unwrap();
            recover_all(root.path(), &manager).unwrap();
            assert_eq!(
                vault::read_note(root.path(), "note.md").unwrap(),
                "external replacement"
            );
            let entries = catalog::load(root.path()).unwrap().resolve().unwrap();
            assert!(entries[&old_id].deleted());
            let created = entries
                .values()
                .find(|entry| !entry.deleted() && entry.path.contains("path conflict"))
                .unwrap();
            assert_ne!(created.id, old_id);
            assert_eq!(
                vault::read_note(root.path(), &created.path).unwrap(),
                "new 🙂"
            );
            let before = manager
                .get_or_create_doc(root.path(), &created.path)
                .unwrap();
            recover_all(root.path(), &manager).unwrap();
            assert_eq!(
                before,
                manager
                    .get_or_create_doc(root.path(), &created.path)
                    .unwrap()
            );
        }
    }

    #[test]
    fn replay_of_a_completed_projection_never_overwrites_a_later_collaborative_edit() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        assert!(
            create_with_hook(root.path(), "empty.md", Some(""), &manager, "a", |at| {
                if at == 3 {
                    bail!("interrupted cleanup");
                }
                Ok(())
            })
            .is_err()
        );
        let journal = fs::read_dir(root.path().join(DIRECTORY))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
            .join("intent.json");
        let intent = parse(&fs::read(&journal).unwrap()).unwrap();
        // Simulate an edit saved by another already-running process before it
        // discovers the journal. The empty seed still has a unique identity.
        let state = STANDARD.decode(intent.state.as_ref().unwrap()).unwrap();
        let doc = decode(&state).unwrap();
        doc.get_or_insert_text("content")
            .push(&mut doc.transact_mut(), "later edit 🙂");
        crate::note_transaction::commit(
            root.path(),
            "empty.md",
            "later edit 🙂",
            &CrdtManager::encode_state(&doc),
        )
        .unwrap();
        recover_all(root.path(), &manager).unwrap();
        assert_eq!(
            vault::read_note(root.path(), "empty.md").unwrap(),
            "later edit 🙂"
        );
        assert_eq!(active_id(root.path(), "empty.md"), intent.id);
        assert!(owns(root.path(), "empty.md", &state).unwrap());
        assert_eq!(
            vault::list_vault_items(root.path())
                .unwrap()
                .iter()
                .filter(|item| !item.is_dir)
                .count(),
            1,
            "a subsequent CRDT edit creates no false conflict copy"
        );
        let after = manager.get_or_create_doc(root.path(), "empty.md").unwrap();
        recover_all(root.path(), &manager).unwrap();
        assert_eq!(
            after,
            manager.get_or_create_doc(root.path(), "empty.md").unwrap()
        );
    }

    #[test]
    fn disk_failure_leaves_the_intent_and_preserves_the_blocking_file() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        assert!(create_with_hook(
            root.path(),
            "folder/note.md",
            Some("durable body"),
            &manager,
            "a",
            |at| {
                if at == 1 {
                    fs::write(root.path().join("folder"), "blocking file")?;
                }
                Ok(())
            }
        )
        .is_err());
        assert_eq!(
            fs::read_to_string(root.path().join("folder")).unwrap(),
            "blocking file"
        );
        assert!(root
            .path()
            .join(DIRECTORY)
            .read_dir()
            .unwrap()
            .next()
            .is_some());
        assert!(recover_all(root.path(), &manager).is_err());
        assert_eq!(
            fs::read_to_string(root.path().join("folder")).unwrap(),
            "blocking file"
        );
        fs::remove_file(root.path().join("folder")).unwrap();
        recover_all(root.path(), &manager).unwrap();
        assert_eq!(
            vault::read_note(root.path(), "folder/note.md").unwrap(),
            "durable body"
        );
    }

    #[test]
    fn actual_process_exit_recovers_note_folder_and_assistant_creation_at_every_phase() {
        for kind in ["note", "folder", "draft"] {
            for phase in 0..=if kind == "folder" { 4 } else { 7 } {
                let root = tempfile::tempdir().unwrap();
                let output = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "creation::tests::crash_worker",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env("LOWNOTES_CREATION_CRASH_ROOT", root.path())
                    .env("LOWNOTES_CREATION_CRASH_KIND", kind)
                    .env("LOWNOTES_CREATION_CRASH_PHASE", phase.to_string())
                    .output()
                    .unwrap();
                assert_eq!(
                    output.status.code(),
                    Some(86),
                    "kind={kind} phase={phase}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                // The normal listing path must complete creation recovery.
                let items = vault::list_vault_items(root.path()).unwrap();
                let manager = CrdtManager::new();
                if kind == "folder" {
                    assert!(items
                        .iter()
                        .any(|item| item.is_dir && item.path == "folder/nested"));
                    active_id(root.path(), "folder/nested");
                } else {
                    assert_eq!(
                        vault::read_note(root.path(), "folder/note.md").unwrap(),
                        "# Durable 🙂\r\n- [ ] Task\r\n"
                    );
                    let state = manager
                        .get_or_create_doc(root.path(), "folder/note.md")
                        .unwrap();
                    assert_eq!(
                        text(&decode(&state).unwrap()),
                        vault::read_note(root.path(), "folder/note.md").unwrap()
                    );
                    let id = active_id(root.path(), "folder/note.md");
                    assert!(structural::load_paths(root.path())
                        .unwrap()
                        .paths
                        .contains_key(&id));
                }
                recover_all(root.path(), &manager).unwrap();
                assert_eq!(
                    fs::read_dir(root.path().join(DIRECTORY)).unwrap().count(),
                    0
                );
            }
        }
    }

    #[test]
    fn an_external_edit_after_projection_preserves_the_complete_creation_as_well() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        assert!(create_with_hook(
            root.path(),
            "note.md",
            Some("original creation"),
            &manager,
            "a",
            |at| {
                if at == 2 {
                    bail!("interrupted");
                }
                Ok(())
            }
        )
        .is_err());
        fs::write(root.path().join("note.md"), "external edit").unwrap();
        filetime::set_file_mtime(
            root.path().join("note.md"),
            filetime::FileTime::from_system_time(
                std::time::SystemTime::now() + std::time::Duration::from_secs(2),
            ),
        )
        .unwrap();
        recover_all(root.path(), &manager).unwrap();
        assert_eq!(
            vault::read_note(root.path(), "note.md").unwrap(),
            "external edit"
        );
        let copy = CrdtManager::conflict_path("note.md", "original creation").unwrap();
        assert_eq!(
            vault::read_note(root.path(), &copy).unwrap(),
            "original creation"
        );
        assert_eq!(
            text(&decode(&manager.get_or_create_doc(root.path(), "note.md").unwrap()).unwrap()),
            "external edit"
        );
    }

    #[test]
    fn recovery_of_an_observed_deleted_creation_preserves_text_without_resurrection() {
        for phase in 1..=3 {
            let root = tempfile::tempdir().unwrap();
            let manager = CrdtManager::new();
            assert!(create_with_hook(
                root.path(),
                "note.md",
                Some("offline creation"),
                &manager,
                "a",
                |at| {
                    if at == phase {
                        bail!("interrupted");
                    }
                    Ok(())
                }
            )
            .is_err());
            let id = active_id(root.path(), "note.md");
            catalog::transact(root.path(), |catalog| {
                catalog.push(
                    "other-device",
                    Change::Delete {
                        id: id.clone(),
                        observed: std::collections::BTreeMap::from([(id.clone(), None)]),
                    },
                )?;
                Ok(())
            })
            .unwrap();
            recover_all(root.path(), &manager).unwrap();
            assert!(!root.path().join("note.md").exists());
            assert!(catalog::load(root.path()).unwrap().resolve().unwrap()[&id].deleted());
            let copies = vault::list_vault_items(root.path())
                .unwrap()
                .into_iter()
                .filter(|item| !item.is_dir)
                .collect::<Vec<_>>();
            assert_eq!(copies.len(), 1);
            assert_eq!(
                vault::read_note(root.path(), &copies[0].path).unwrap(),
                "offline creation"
            );
            assert!(crate::catalog_sync::has_restorable(root.path()).unwrap());
            let restored = crate::catalog_sync::restore_latest(root.path(), &manager, "a")
                .unwrap()
                .unwrap();
            assert_eq!(
                vault::read_note(root.path(), &restored.0).unwrap(),
                "offline creation"
            );
            assert_eq!(active_id(root.path(), &restored.0), id);
        }
    }

    #[test]
    fn a_catalog_move_during_creation_follows_the_identity_without_leaving_an_old_file() {
        for phase in [1, 2] {
            let root = tempfile::tempdir().unwrap();
            let manager = CrdtManager::new();
            assert!(create_with_hook(
                root.path(),
                "note.md",
                Some("created before the move"),
                &manager,
                "a",
                |at| {
                    if at == phase {
                        bail!("interrupted");
                    }
                    Ok(())
                }
            )
            .is_err());
            let id = active_id(root.path(), "note.md");
            catalog::transact(root.path(), |catalog| {
                catalog.push(
                    "other-device",
                    Change::Move {
                        id: id.clone(),
                        location: Location {
                            parent: None,
                            name: "moved.md".into(),
                        },
                    },
                )?;
                Ok(())
            })
            .unwrap();
            recover_all(root.path(), &manager).unwrap();
            assert!(!root.path().join("note.md").exists());
            assert_eq!(
                vault::read_note(root.path(), "moved.md").unwrap(),
                "created before the move"
            );
            assert_eq!(active_id(root.path(), "moved.md"), id);
            let state = manager.get_or_create_doc(root.path(), "moved.md").unwrap();
            assert_eq!(text(&decode(&state).unwrap()), "created before the move");
        }
    }

    #[test]
    fn native_save_cannot_recreate_the_old_path_while_a_move_is_running() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        create(root.path(), "note.md", Some("original"), &manager, "a").unwrap();
        let (entered, ready) = std::sync::mpsc::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let move_root = root.path().to_path_buf();
        let move_manager = manager.clone();
        let moving = std::thread::spawn(move || {
            structural::exclusive(&move_root, &move_manager, || {
                entered.send(()).unwrap();
                gate.recv().unwrap();
                structural::rename(&move_root, "note.md", "moved.md", &move_manager, "a")
            })
        });
        ready.recv().unwrap();
        let (completed, observed) = std::sync::mpsc::channel();
        let save_root = root.path().to_path_buf();
        let saving = std::thread::spawn(move || {
            let result = manager.replace_note_text(&save_root, "note.md", "late native save");
            completed.send(result.is_err()).unwrap();
        });
        assert!(observed
            .recv_timeout(std::time::Duration::from_millis(30))
            .is_err());
        release.send(()).unwrap();
        moving.join().unwrap().unwrap();
        assert!(observed
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap());
        saving.join().unwrap();
        assert!(!root.path().join("note.md").exists());
        assert_eq!(
            vault::read_note(root.path(), "moved.md").unwrap(),
            "original"
        );
    }

    #[test]
    #[ignore = "Isolated child process for abrupt creation exit regression"]
    fn crash_worker() {
        let root = PathBuf::from(std::env::var_os("LOWNOTES_CREATION_CRASH_ROOT").unwrap());
        match std::env::var("LOWNOTES_CREATION_CRASH_KIND")
            .unwrap()
            .as_str()
        {
            "note" => {
                vault::create_note(
                    &root,
                    "folder/note.md",
                    Some("# Durable 🙂\r\n- [ ] Task\r\n"),
                    "en-US",
                )
                .unwrap();
            }
            "folder" => {
                vault::create_folder(&root, "folder/nested").unwrap();
            }
            "draft" => {
                crate::assistant::save_draft(
                    &root,
                    &crate::assistant::NoteDraft {
                        path: "folder/note.md".into(),
                        content: "# Durable 🙂\r\n- [ ] Task\r\n".into(),
                    },
                )
                .unwrap();
            }
            _ => panic!("unexpected fixture kind"),
        }
        panic!("fixture did not exit at the requested phase");
    }
}
