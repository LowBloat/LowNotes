//! Immutable local checkpoints follow the note identity, not its filename.
//! Restoring text is a new edit to the current document, never a state rollback.
use crate::{
    catalog,
    crdt::{AppliedUpdate, CrdtManager},
    storage, structural, vault,
};
use anyhow::{bail, Context};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

const DIRECTORY: &str = ".lownotes/versions";
const INTERVAL_MS: u64 = 60_000;
static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static RECENT: OnceLock<Mutex<HashMap<PathBuf, u64>>> = OnceLock::new();

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Version {
    version: u8,
    id: String,
    note_id: String,
    path: String,
    created_ms: u64,
    hash: String,
    content: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct VersionSummary {
    pub id: String,
    pub note_id: String,
    pub path: String,
    pub created_ms: u64,
    pub hash: String,
    pub characters: usize,
}
#[derive(Clone, Debug, Serialize)]
pub struct VersionContent {
    pub summary: VersionSummary,
    pub content: String,
}
impl Version {
    fn summary(&self) -> VersionSummary {
        VersionSummary {
            id: self.id.clone(),
            note_id: self.note_id.clone(),
            path: self.path.clone(),
            created_ms: self.created_ms,
            hash: self.hash.clone(),
            characters: self.content.chars().count(),
        }
    }
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn version_path(root: &Path, note: &str, version: &str) -> anyhow::Result<PathBuf> {
    if !valid_id(note) || !valid_id(version) {
        bail!("invalid note version identity");
    }
    Ok(root
        .join(DIRECTORY)
        .join(note)
        .join(format!("{version}.json")))
}
fn parse(bytes: &[u8], path: &Path) -> anyhow::Result<Version> {
    if bytes.len() > 64 * 1024 * 1024 {
        bail!("note version is too large");
    }
    let version: Version = serde_json::from_slice(bytes)?;
    if version.version != 1
        || !valid_id(&version.id)
        || !valid_id(&version.note_id)
        || path.file_name().and_then(|part| part.to_str()) != Some(&format!("{}.json", version.id))
        || path
            .parent()
            .and_then(Path::file_name)
            .and_then(|part| part.to_str())
            != Some(&version.note_id)
        || !vault::is_markdown(Path::new(&version.path))
        || version.path.len() > 4096
        || version.path.split('/').any(|part| {
            part.is_empty()
                || part.starts_with('.')
                || part.contains(':')
                || part.contains('\\')
                || part.chars().any(char::is_control)
        })
        || version.content.len() as u64 > vault::MAX_NOTE_BYTES
        || version.created_ms == 0
        || version.hash != blake3::hash(version.content.as_bytes()).to_hex().as_str()
    {
        bail!("invalid note version");
    }
    Ok(version)
}
fn read_file(path: &Path) -> anyhow::Result<Option<Version>> {
    if fs::metadata(path).is_ok_and(|metadata| metadata.len() > 64 * 1024 * 1024) {
        storage::report_recovery(path, false);
        bail!("note version is too large");
    }
    storage::read_validated(path, |bytes| parse(bytes, path).is_ok())?
        .map(|bytes| parse(&bytes, path))
        .transpose()
}
fn all(root: &Path, note: &str) -> anyhow::Result<Vec<VersionSummary>> {
    if !valid_id(note) {
        bail!("invalid note identity");
    }
    let directory = root.join(DIRECTORY).join(note);
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut versions = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".json") || !valid_id(name.trim_end_matches(".json")) {
            continue;
        }
        if let Some(version) = read_file(&entry.path())? {
            versions.push(version.summary());
        }
    }
    versions.sort_by(|a, b| (b.created_ms, &b.id).cmp(&(a.created_ms, &a.id)));
    Ok(versions)
}

/// Called before a Markdown/CRDT intent is published. At most one ordinary
/// checkpoint per minute; explicit restore/merge captures its prior text too.
pub(crate) fn record_previous(
    root: &Path,
    path: &str,
    before: &str,
    replacement: &str,
    force: bool,
) -> anyhow::Result<()> {
    if before == replacement {
        return Ok(());
    }
    if before.len() as u64 > vault::MAX_NOTE_BYTES {
        bail!("errors.noteTooLarge");
    }
    let key = vault::safe_join(root, path)?;
    let now = now_ms();
    if !force
        && RECENT
            .get_or_init(Mutex::default)
            .lock()
            .get(&key)
            .is_some_and(|last| now.saturating_sub(*last) < INTERVAL_MS)
    {
        return Ok(());
    }
    let _guard = LOCK.get_or_init(Mutex::default).lock();
    let id = catalog::load(root)?
        .resolve()?
        .into_values()
        .find(|entry| !entry.deleted() && !entry.is_dir && entry.path == path)
        .map(|entry| entry.id)
        .unwrap_or_else(|| {
            blake3::hash(format!("note:{path}").as_bytes())
                .to_hex()
                .to_string()
        });
    let hash = blake3::hash(before.as_bytes()).to_hex().to_string();
    let versions = all(root, &id)?;
    let latest = versions.first();
    if latest.is_some_and(|last| {
        last.hash == hash || (!force && now.saturating_sub(last.created_ms) < INTERVAL_MS)
    }) {
        let mut recent = RECENT.get_or_init(Mutex::default).lock();
        if recent.len() >= 256 {
            recent.clear();
        }
        recent.insert(key, latest.unwrap().created_ms);
        return Ok(());
    }
    let version = Version {
        version: 1,
        id: catalog::random_id(),
        note_id: id,
        path: path.into(),
        created_ms: now,
        hash,
        content: before.into(),
    };
    let file = version_path(root, &version.note_id, &version.id)?;
    storage::write_without_backup(&file, &serde_json::to_vec(&version)?, |bytes| {
        parse(bytes, &file).is_ok()
    })?;
    let mut recent = RECENT.get_or_init(Mutex::default).lock();
    if recent.len() >= 256 {
        recent.clear();
    }
    recent.insert(key, now);
    Ok(())
}

pub(crate) fn forget(root: &Path, path: &str) -> anyhow::Result<()> {
    RECENT
        .get_or_init(Mutex::default)
        .lock()
        .remove(&vault::safe_join(root, path)?);
    Ok(())
}

pub fn list(root: &Path, note: &str) -> anyhow::Result<Vec<VersionSummary>> {
    let _guard = LOCK.get_or_init(Mutex::default).lock();
    all(root, note)
}
pub fn get(root: &Path, note: &str, version: &str) -> anyhow::Result<VersionContent> {
    let _guard = LOCK.get_or_init(Mutex::default).lock();
    let version =
        read_file(&version_path(root, note, version)?)?.context("note version is unavailable")?;
    Ok(VersionContent {
        summary: version.summary(),
        content: version.content,
    })
}

pub(crate) fn expire(
    root: &Path,
    cutoff: Option<u64>,
    apply: bool,
    report: &mut crate::retention::CleanupReport,
) -> anyhow::Result<()> {
    let Some(cutoff) = cutoff else {
        return Ok(());
    };
    let _guard = LOCK.get_or_init(Mutex::default).lock();
    let directory = root.join(DIRECTORY);
    if !directory.exists() {
        return Ok(());
    }
    if !crate::retention::owned_path(root, &directory, false)? {
        report.protected += 1;
        return Ok(());
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if !entry.file_type()?.is_dir() || !valid_id(&id) {
            continue;
        }
        if !crate::retention::owned_path(root, &entry.path(), false)? {
            report.protected += 1;
            continue;
        }
        for file in fs::read_dir(entry.path())? {
            let file = file?;
            let name = file.file_name().to_string_lossy().into_owned();
            if !file.file_type()?.is_file()
                || !name.ends_with(".json")
                || !valid_id(name.trim_end_matches(".json"))
            {
                continue;
            }
            if !crate::retention::owned_path(root, &file.path(), false)? {
                report.protected += 1;
                continue;
            }
            let Some(version) = read_file(&file.path())? else {
                continue;
            };
            if version.created_ms >= cutoff {
                continue;
            }
            report.versions += 1;
            if apply {
                storage::remove_file(&storage::backup_path(&file.path()))?;
                storage::remove_file(&file.path())?;
            }
        }
    }
    Ok(())
}

pub fn apply(
    root: &Path,
    note: &str,
    content: &str,
    expected_hash: &str,
    manager: &CrdtManager,
) -> anyhow::Result<(String, AppliedUpdate)> {
    if !valid_id(note) || !valid_id(expected_hash) {
        bail!("invalid version application");
    }
    if content.len() as u64 > vault::MAX_NOTE_BYTES {
        bail!("errors.noteTooLarge");
    }
    structural::exclusive(root, manager, || {
        let entry = catalog::load(root)?
            .resolve()?
            .remove(note)
            .context("note identity is unavailable")?;
        if entry.deleted() || entry.is_dir {
            bail!("errors.noteDeleted");
        }
        let current = vault::read_note(root, &entry.path)?;
        if blake3::hash(current.as_bytes()).to_hex().as_str() != expected_hash {
            bail!("history.noteChanged");
        }
        record_previous(root, &entry.path, &current, content, true)?;
        let result = manager.replace_note_text(root, &entry.path, content)?;
        Ok((entry.path, result))
    })
}
pub fn restore(
    root: &Path,
    note: &str,
    version: &str,
    expected_hash: &str,
    manager: &CrdtManager,
) -> anyhow::Result<(String, AppliedUpdate)> {
    let version = get(root, note, version)?;
    apply(root, note, &version.content, expected_hash, manager)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::{updates::decoder::Decode, Doc, GetString, ReadTxn, Text, Transact, Update};
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
    fn a_reused_filename_gets_its_own_first_checkpoint_even_in_the_same_minute() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        crate::creation::create(root.path(), "note.md", Some("old initial"), &manager, "a")
            .unwrap();
        let old = active_id(root.path(), "note.md");
        manager
            .replace_note_text(root.path(), "note.md", "old edited")
            .unwrap();
        crate::catalog_sync::delete(root.path(), "note.md", &manager, "a").unwrap();
        crate::creation::create(root.path(), "note.md", Some("new initial"), &manager, "a")
            .unwrap();
        let new = active_id(root.path(), "note.md");
        manager
            .replace_note_text(root.path(), "note.md", "new edited")
            .unwrap();
        let versions = list(root.path(), &new).unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(
            get(root.path(), &new, &versions[0].id).unwrap().content,
            "new initial"
        );
        assert_ne!(old, new);
        assert_eq!(list(root.path(), &old).unwrap().len(), 1);
    }
    #[test]
    fn restoration_merges_with_an_unseen_peer_edit_without_rolling_back_collaborative_history() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        crate::creation::create(root.path(), "note.md", Some("original"), &manager, "a").unwrap();
        let id = active_id(root.path(), "note.md");
        manager
            .replace_note_text(root.path(), "note.md", "current")
            .unwrap();
        let peer = Doc::new();
        peer.transact_mut()
            .apply_update(
                Update::decode_v1(&manager.get_or_create_doc(root.path(), "note.md").unwrap())
                    .unwrap(),
            )
            .unwrap();
        let text = peer.get_or_insert_text("content");
        text.push(&mut peer.transact_mut(), " + offline peer");
        let version = list(root.path(), &id).unwrap().remove(0);
        let (_, restored) = restore(
            root.path(),
            &id,
            &version.id,
            &blake3::hash(b"current").to_hex().to_string(),
            &manager,
        )
        .unwrap();
        peer.transact_mut()
            .apply_update(Update::decode_v1(&restored.state).unwrap())
            .unwrap();
        let merged = text.get_string(&peer.transact());
        assert!(merged.contains("original"));
        assert!(merged.contains("offline peer"));
        let update = manager
            .apply_update(root.path(), "note.md", &CrdtManager::encode_state(&peer))
            .unwrap();
        peer.transact_mut()
            .apply_update(Update::decode_v1(&update.state).unwrap())
            .unwrap();
        assert_eq!(
            vault::read_note(root.path(), "note.md").unwrap(),
            text.get_string(&peer.transact())
        );
        assert_eq!(active_id(root.path(), "note.md"), id);
    }
    #[test]
    fn versions_survive_restart_and_moves_and_restore_as_a_new_collaborative_edit() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        crate::creation::create(root.path(), "note.md", Some("original 🙂"), &manager, "a")
            .unwrap();
        let id = active_id(root.path(), "note.md");
        manager
            .replace_note_text(root.path(), "note.md", "edited 🙂")
            .unwrap();
        let versions = list(root.path(), &id).unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(
            get(root.path(), &id, &versions[0].id).unwrap().content,
            "original 🙂"
        );
        structural::rename(root.path(), "note.md", "moved.md", &manager, "a").unwrap();
        let manager = CrdtManager::new();
        let before = manager.get_or_create_doc(root.path(), "moved.md").unwrap();
        let before_doc = Doc::new();
        before_doc
            .transact_mut()
            .apply_update(Update::decode_v1(&before).unwrap())
            .unwrap();
        let expected = blake3::hash(b"edited \xf0\x9f\x99\x82")
            .to_hex()
            .to_string();
        let (path, restored) =
            restore(root.path(), &id, &versions[0].id, &expected, &manager).unwrap();
        assert_eq!(path, "moved.md");
        assert_eq!(vault::read_note(root.path(), &path).unwrap(), "original 🙂");
        let after_doc = Doc::new();
        after_doc
            .transact_mut()
            .apply_update(Update::decode_v1(&restored.state).unwrap())
            .unwrap();
        let after = after_doc.transact().state_vector();
        assert!(before_doc
            .transact()
            .state_vector()
            .iter()
            .all(|(client, clock)| after.get(client) >= *clock));
        assert!(list(root.path(), &id).unwrap().iter().any(|version| get(
            root.path(),
            &id,
            &version.id
        )
        .unwrap()
        .content
            == "edited 🙂"));
        assert_eq!(active_id(root.path(), &path), id);
    }
    #[test]
    fn ordinary_typing_is_coalesced_but_explicit_merge_captures_its_previous_text() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        crate::creation::create(root.path(), "note.md", Some("start"), &manager, "a").unwrap();
        let id = active_id(root.path(), "note.md");
        for index in 0..10 {
            manager
                .replace_note_text(root.path(), "note.md", &format!("typed {index}"))
                .unwrap();
        }
        assert_eq!(list(root.path(), &id).unwrap().len(), 1);
        apply(
            root.path(),
            &id,
            "merged result",
            &blake3::hash(b"typed 9").to_hex().to_string(),
            &manager,
        )
        .unwrap();
        assert_eq!(list(root.path(), &id).unwrap().len(), 2);
        assert_eq!(
            vault::read_note(root.path(), "note.md").unwrap(),
            "merged result"
        );
    }
    #[test]
    fn changed_text_and_filename_replacement_cannot_receive_a_stale_restoration() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        crate::creation::create(root.path(), "note.md", Some("start"), &manager, "a").unwrap();
        let id = active_id(root.path(), "note.md");
        manager
            .replace_note_text(root.path(), "note.md", "new text")
            .unwrap();
        let version = list(root.path(), &id).unwrap().remove(0);
        assert!(restore(
            root.path(),
            &id,
            &version.id,
            &blake3::hash(b"start").to_hex().to_string(),
            &manager
        )
        .is_err());
        assert_eq!(
            vault::read_note(root.path(), "note.md").unwrap(),
            "new text"
        );
        crate::catalog_sync::delete(root.path(), "note.md", &manager, "a").unwrap();
        crate::creation::create(root.path(), "note.md", Some("replacement"), &manager, "a")
            .unwrap();
        assert_ne!(active_id(root.path(), "note.md"), id);
        assert!(restore(
            root.path(),
            &id,
            &version.id,
            &blake3::hash(b"replacement").to_hex().to_string(),
            &manager
        )
        .is_err());
        assert_eq!(
            vault::read_note(root.path(), "note.md").unwrap(),
            "replacement"
        );
    }
}
