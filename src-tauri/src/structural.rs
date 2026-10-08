//! Recoverable filesystem moves retain the exact Yrs history. The durable
//! intent precedes staging the source and remains until all projections exist.
use crate::{
    catalog::{self, Catalog, Change, Location},
    crdt::CrdtManager,
    storage, vault,
};
use anyhow::{bail, Context};
use parking_lot::ReentrantMutex;
use serde::{Deserialize, Serialize};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};
use yrs::{updates::decoder::Decode, Doc, GetString, Transact, Update};

const DIRECTORY: &str = ".lownotes/pending-moves";
const BINDINGS: &str = ".lownotes/catalog-paths.json";
static LOCK: OnceLock<ReentrantMutex<()>> = OnceLock::new();
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
struct StateRef {
    old: String,
    new: String,
}

#[derive(Serialize, Deserialize)]
struct MoveIntent {
    version: u8,
    old: String,
    new: String,
    is_dir: bool,
    entry_id: String,
    states: Vec<StateRef>,
    catalog: Catalog,
    before_paths: BTreeMap<String, String>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct MaterializedPaths {
    #[serde(default)]
    pub paths: BTreeMap<String, String>,
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.starts_with('/')
        && !path
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '\\' | ':'))
        && path
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.'))
}

fn moved_path(path: &str, old: &str, new: &str) -> Option<String> {
    if path == old {
        Some(new.into())
    } else {
        path.strip_prefix(&format!("{old}/"))
            .map(|suffix| format!("{new}/{suffix}"))
    }
}

fn parse(bytes: &[u8]) -> anyhow::Result<MoveIntent> {
    let intent: MoveIntent = serde_json::from_slice(bytes)?;
    if intent.version != 1
        || !valid_path(&intent.old)
        || !valid_path(&intent.new)
        || intent.old == intent.new
        || (intent.is_dir && intent.new.starts_with(&format!("{}/", intent.old)))
    {
        bail!("invalid move intent");
    }
    let state = intent.catalog.resolve()?;
    let item = state
        .get(&intent.entry_id)
        .context("missing moved identity")?;
    if item.path != intent.new || item.is_dir != intent.is_dir || item.deleted() {
        bail!("move intent does not match catalog");
    }
    let mut notes = BTreeSet::new();
    for entry in &intent.states {
        if !notes.insert(&entry.old)
            || moved_path(&entry.old, &intent.old, &intent.new).as_deref() != Some(&entry.new)
            || !valid_path(&entry.old)
            || !valid_path(&entry.new)
            || !vault::is_markdown(Path::new(&entry.old))
        {
            bail!("invalid moved state reference");
        }
    }
    if intent
        .before_paths
        .iter()
        .any(|(id, path)| !state.contains_key(id) || !valid_path(path))
    {
        bail!("invalid previous catalog paths");
    }
    Ok(intent)
}

fn state_path(directory: &Path, old_note: &str) -> PathBuf {
    directory.join("states").join(format!(
        "{}.bin",
        blake3::hash(old_note.as_bytes()).to_hex()
    ))
}

fn valid_state(note: &str, bytes: &[u8]) -> bool {
    CrdtManager::decode_file(&CrdtManager::state_relative_path(note), bytes).is_ok_and(
        |(_, update)| {
            let doc = Doc::new();
            Update::decode_v1(update)
                .is_ok_and(|update| doc.transact_mut().apply_update(update).is_ok())
        },
    )
}

pub fn load_paths(root: &Path) -> anyhow::Result<MaterializedPaths> {
    let valid = |bytes: &[u8]| {
        serde_json::from_slice::<MaterializedPaths>(bytes).is_ok_and(|state| {
            state.paths.iter().all(|(id, path)| {
                id.len() == 64
                    && id.bytes().all(|byte| byte.is_ascii_hexdigit())
                    && valid_path(path)
            })
        })
    };
    match storage::read_validated(&root.join(BINDINGS), valid)? {
        Some(bytes) => Ok(serde_json::from_slice(&bytes)?),
        None => Ok(MaterializedPaths::default()),
    }
}

pub(crate) fn save_paths(root: &Path, paths: &MaterializedPaths) -> anyhow::Result<()> {
    storage::write_validated(
        &root.join(BINDINGS),
        &serde_json::to_vec_pretty(paths)?,
        |bytes| {
            serde_json::from_slice::<MaterializedPaths>(bytes).is_ok_and(|state| {
                state.paths.iter().all(|(id, path)| {
                    id.len() == 64
                        && id.bytes().all(|byte| byte.is_ascii_hexdigit())
                        && valid_path(path)
                })
            })
        },
    )
}

pub(crate) fn exclusive<T>(
    root: &Path,
    manager: &CrdtManager,
    action: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let _guard = LOCK.get_or_init(|| ReentrantMutex::new(())).lock();
    let _scope = Scope::enter();
    crate::note_transaction::recover_all(root)?;
    recover_inner(root, manager)?;
    action()
}

fn finish(
    root: &Path,
    directory: &Path,
    intent: &MoveIntent,
    manager: &CrdtManager,
    recovered: bool,
    hook: &impl Fn(u8) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    catalog::transact(root, |current| {
        *current = current.merged(&intent.catalog)?;
        Ok(())
    })?;
    let source = vault::safe_join(root, &intent.old)?;
    let destination = vault::safe_join(root, &intent.new)?;
    let staged = directory.join("item");
    if source.exists() {
        if staged.exists() {
            bail!("move source and staged copy both exist");
        }
        fs::rename(&source, &staged)?;
        hook(1)?;
    }
    if staged.exists() {
        if destination.exists() {
            bail!("errors.targetExists");
        }
        fs::create_dir_all(
            destination
                .parent()
                .context("missing move destination parent")?,
        )?;
        fs::rename(&staged, &destination)?;
        hook(2)?;
    }
    if !destination.exists() {
        bail!("move has no recoverable source or destination");
    }
    crate::note_transaction::recover_all(root)?;
    for reference in &intent.states {
        let saved = state_path(directory, &reference.old);
        let original =
            match storage::read_validated(&saved, |bytes| valid_state(&reference.old, bytes)) {
                Ok(Some(bytes)) => bytes,
                _ => CrdtManager::read_state_file(
                    root,
                    &CrdtManager::state_relative_path(&reference.old),
                )?
                .context("moved collaborative history is unavailable")?,
            };
        let (_, update) =
            CrdtManager::decode_file(&CrdtManager::state_relative_path(&reference.old), &original)?;
        let doc = Doc::new();
        doc.transact_mut()
            .apply_update(Update::decode_v1(update)?)?;
        let content = vault::read_note(root, &reference.new)?;
        if let Some(bytes) =
            CrdtManager::read_state_file(root, &CrdtManager::state_relative_path(&reference.new))?
        {
            let (_, current) = CrdtManager::decode_file(
                &CrdtManager::state_relative_path(&reference.new),
                &bytes,
            )?;
            let previous = Doc::new();
            previous
                .transact_mut()
                .apply_update(Update::decode_v1(current)?)?;
            let previous_text = previous
                .get_or_insert_text("content")
                .get_string(&previous.transact());
            if previous_text != content && !previous_text.is_empty() {
                let copy = CrdtManager::conflict_path(&reference.new, &previous_text)?;
                storage::write_text(&vault::safe_join(root, &copy)?, &previous_text)?;
                storage::report_recovery(&vault::safe_join(root, &copy)?, true);
            }
            doc.transact_mut()
                .apply_update(Update::decode_v1(current)?)?;
        }
        CrdtManager::replace_text(&doc, &content);
        crate::note_transaction::commit(
            root,
            &reference.new,
            &content,
            &CrdtManager::encode_state(&doc),
        )?;
    }
    hook(3)?;
    let mut paths = load_paths(root)?;
    for (id, old_path) in &intent.before_paths {
        paths
            .paths
            .entry(id.clone())
            .or_insert_with(|| old_path.clone());
        if let Some(new_path) = moved_path(old_path, &intent.old, &intent.new) {
            paths.paths.insert(id.clone(), new_path);
        }
    }
    save_paths(root, &paths)?;
    for reference in &intent.states {
        let old_state = root.join(CrdtManager::state_relative_path(&reference.old));
        storage::remove_file(&storage::backup_path(&old_state))?;
        storage::remove_file(&old_state)?;
    }
    manager.invalidate_path(root, &intent.old)?;
    manager.invalidate_path(root, &intent.new)?;
    // Only this owned transaction directory is removed, after the staged user
    // item has moved out and every Markdown/CRDT projection is durable.
    let file = directory.join("intent.json");
    storage::remove_file(&storage::backup_path(&file))?;
    storage::remove_file(&file)?;
    fs::remove_dir_all(directory)?;
    if recovered {
        storage::report_recovery(&destination, true);
    }
    Ok(())
}

fn recover_inner(root: &Path, manager: &CrdtManager) -> anyhow::Result<()> {
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
            continue;
        };
        let intent = parse(&bytes)?;
        if let Err(error) = finish(root, &entry.path(), &intent, manager, true, &|_| Ok(())) {
            storage::report_recovery(&file, false);
            return Err(error);
        }
    }
    Ok(())
}

pub fn recover_all(root: &Path, manager: &CrdtManager) -> anyhow::Result<()> {
    if BUSY.get() {
        return Ok(());
    }
    let _guard = LOCK.get_or_init(|| ReentrantMutex::new(())).lock();
    let _scope = Scope::enter();
    recover_inner(root, manager)
}

pub fn rename(
    root: &Path,
    old: &str,
    new: &str,
    manager: &CrdtManager,
    author: &str,
) -> anyhow::Result<()> {
    rename_with_hook(root, old, new, manager, author, |_| Ok(()))
}

fn rename_with_hook(
    root: &Path,
    old: &str,
    new: &str,
    manager: &CrdtManager,
    author: &str,
    hook: impl Fn(u8) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let _guard = LOCK.get_or_init(|| ReentrantMutex::new(())).lock();
    let _scope = Scope::enter();
    crate::note_transaction::recover_all(root)?;
    recover_inner(root, manager)?;
    if !valid_path(old) || !valid_path(new) || old == new {
        bail!("errors.pathEscape");
    }
    let source = vault::safe_join(root, old)?;
    let destination = vault::safe_join(root, new)?;
    let metadata = source.symlink_metadata().context("errors.sourceNotFound")?;
    if metadata.is_symlink() {
        bail!("errors.undoUnsupportedItem");
    }
    let is_dir = metadata.is_dir();
    if is_dir && destination.starts_with(&source) {
        bail!("errors.pathEscape");
    }
    if destination.exists() && fs::canonicalize(&source)? != fs::canonicalize(&destination)? {
        bail!("errors.targetExists");
    }
    let mut note_paths = Vec::new();
    for entry in walkdir::WalkDir::new(&source)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !entry.file_name().to_str().unwrap_or("").starts_with('.'))
    {
        let entry = entry?;
        if entry.file_type().is_file() && vault::is_markdown(entry.path()) {
            let path = entry
                .path()
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            manager.get_or_create_doc(root, &path)?;
            note_paths.push(path);
        }
    }
    let mut planned = catalog::load(root)?;
    planned.discover_existing(root, author)?;
    let before_paths: BTreeMap<_, _> = planned
        .resolve()?
        .into_iter()
        .filter(|(_, entry)| !entry.deleted())
        .map(|(id, entry)| (id, entry.path))
        .collect();
    let entry_id = planned.ensure_path(old, is_dir, author)?;
    let (parent_path, name) = new
        .rsplit_once('/')
        .map(|(parent, name)| (Some(parent), name))
        .unwrap_or((None, new));
    let parent = parent_path
        .map(|path| planned.ensure_path(path, true, author))
        .transpose()?;
    planned.push(
        author,
        Change::Move {
            id: entry_id.clone(),
            location: Location {
                parent,
                name: name.into(),
            },
        },
    )?;
    let directory = root.join(DIRECTORY).join(catalog::random_id());
    fs::create_dir_all(directory.join("states"))?;
    let mut states = Vec::new();
    for note in note_paths {
        let bytes = CrdtManager::read_state_file(root, &CrdtManager::state_relative_path(&note))?
            .context("missing collaborative state before move")?;
        storage::write_validated(&state_path(&directory, &note), &bytes, |bytes| {
            valid_state(&note, bytes)
        })?;
        states.push(StateRef {
            new: moved_path(&note, old, new).context("invalid moved note")?,
            old: note,
        });
    }
    let intent = MoveIntent {
        version: 1,
        old: old.into(),
        new: new.into(),
        is_dir,
        entry_id,
        states,
        catalog: planned,
        before_paths,
    };
    storage::write_validated(
        &directory.join("intent.json"),
        &serde_json::to_vec_pretty(&intent)?,
        |bytes| parse(bytes).is_ok(),
    )?;
    hook(0)?;
    finish(root, &directory, &intent, manager, false, &hook)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::{ReadTxn, StateVector, Text};
    #[test]
    fn folder_moves_keep_crdt_history_assets_and_identity_after_every_interrupted_phase() {
        for phase in 0..4 {
            let root = tempfile::tempdir().unwrap();
            fs::create_dir_all(root.path().join("folder/sub")).unwrap();
            fs::write(root.path().join("folder/sub/note.md"), "# Original 🙂\n").unwrap();
            fs::write(root.path().join("folder/asset.bin"), [0, 255, 23]).unwrap();
            let manager = CrdtManager::new();
            let before = manager
                .get_or_create_doc(root.path(), "folder/sub/note.md")
                .unwrap();
            assert!(
                rename_with_hook(root.path(), "folder", "moved", &manager, "A", |step| {
                    if step == phase {
                        bail!("simulated interrupted move");
                    }
                    Ok(())
                })
                .is_err()
            );
            recover_all(root.path(), &CrdtManager::new()).unwrap();
            assert!(!root.path().join("folder").exists());
            assert_eq!(
                fs::read(root.path().join("moved/asset.bin")).unwrap(),
                [0, 255, 23]
            );
            assert_eq!(
                fs::read_to_string(root.path().join("moved/sub/note.md")).unwrap(),
                "# Original 🙂\n"
            );
            let after = CrdtManager::new()
                .get_or_create_doc(root.path(), "moved/sub/note.md")
                .unwrap();
            assert_eq!(before, after, "the original Yjs IDs survive the move");
            let catalog = catalog::load(root.path()).unwrap();
            let identity = blake3::hash(b"note:folder/sub/note.md")
                .to_hex()
                .to_string();
            assert_eq!(
                catalog.resolve().unwrap()[&identity].path,
                "moved/sub/note.md"
            );
            assert_eq!(
                load_paths(root.path()).unwrap().paths[&identity],
                "moved/sub/note.md"
            );
            assert!(!root
                .path()
                .join(CrdtManager::state_relative_path("folder/sub/note.md"))
                .exists());
        }
    }

    #[test]
    fn edits_created_before_a_move_still_merge_after_the_move_and_native_replacement() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a.md"), "baseline\n").unwrap();
        let manager = CrdtManager::new();
        let before = manager.get_or_create_doc(root.path(), "a.md").unwrap();
        let remote = Doc::with_client_id(1234);
        remote
            .transact_mut()
            .apply_update(Update::decode_v1(&before).unwrap())
            .unwrap();
        remote
            .get_or_insert_text("content")
            .push(&mut remote.transact_mut(), "remote\n");
        rename(root.path(), "a.md", "notes/new.md", &manager, "A").unwrap();
        manager
            .apply_update(
                root.path(),
                "notes/new.md",
                &remote.transact().encode_diff_v1(&StateVector::default()),
            )
            .unwrap();
        assert_eq!(
            vault::read_note(root.path(), "notes/new.md").unwrap(),
            "baseline\nremote\n"
        );
        let saved = manager
            .replace_note_text(root.path(), "notes/new.md", "baseline\nremote\nlocal 🙂\n")
            .unwrap();
        assert!(saved.changed);
        let restored = Doc::new();
        restored
            .transact_mut()
            .apply_update(Update::decode_v1(&saved.state).unwrap())
            .unwrap();
        restored
            .transact_mut()
            .apply_update(Update::decode_v1(&before).unwrap())
            .unwrap();
        assert_eq!(
            restored
                .get_or_insert_text("content")
                .get_string(&restored.transact()),
            "baseline\nremote\nlocal 🙂\n"
        );
        assert!(
            !manager
                .replace_note_text(root.path(), "notes/new.md", "baseline\nremote\nlocal 🙂\n")
                .unwrap()
                .changed
        );
        assert!(manager
            .replace_note_text(root.path(), "a.md", "ghost")
            .is_err());
    }

    #[test]
    fn recovery_preserves_a_target_edited_after_the_move_and_rejects_an_unrelated_destination() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a.md"), "original").unwrap();
        let manager = CrdtManager::new();
        manager.get_or_create_doc(root.path(), "a.md").unwrap();
        assert!(
            rename_with_hook(root.path(), "a.md", "b.md", &manager, "A", |step| {
                if step == 2 {
                    bail!("interrupted");
                }
                Ok(())
            })
            .is_err()
        );
        fs::write(root.path().join("b.md"), "new external text").unwrap();
        recover_all(root.path(), &manager).unwrap();
        assert_eq!(
            vault::read_note(root.path(), "b.md").unwrap(),
            "new external text"
        );
        assert!(rename(root.path(), "b.md", "b.md/child.md", &manager, "A").is_err());
        fs::write(root.path().join("c.md"), "other note").unwrap();
        assert!(rename(root.path(), "b.md", "c.md", &manager, "A").is_err());
        assert_eq!(vault::read_note(root.path(), "c.md").unwrap(), "other note");
    }
}
