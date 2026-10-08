//! Materialize structural operations before exchanging note contents. An intent
//! stages every affected file before placing any destination, so swaps, nested
//! directory moves and deletions remain recoverable. Deleted data stays on disk.
use crate::{
    catalog::{self, Catalog, Change, Location, ResolvedEntry},
    crdt::CrdtManager,
    storage, structural, vault,
};
use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use yrs::{updates::decoder::Decode, Doc, GetString, Transact, Update};

const PENDING: &str = ".lownotes/pending-structure";
const PENDING_CATALOG: &str = ".lownotes/pending-catalog.json";
pub const TRASH: &str = ".lownotes/trash";
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

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Item {
    id: String,
    old: String,
    new: Option<String>,
    is_dir: bool,
    location: Location,
    #[serde(default)]
    restore_from: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Intent {
    version: u8,
    created_ms: u64,
    device: String,
    catalog: Catalog,
    before: BTreeMap<String, String>,
    items: Vec<Item>,
}
#[derive(Default, Serialize, Deserialize)]
struct Progress {
    staged: bool,
    placed: BTreeSet<String>,
}

#[derive(Serialize, Deserialize)]
struct PendingCatalog {
    version: u8,
    device: String,
    catalog: Catalog,
}

fn parse_pending(bytes: &[u8]) -> anyhow::Result<PendingCatalog> {
    if bytes.len() > 24 * 1024 * 1024 {
        bail!("pending catalog is too large");
    }
    let pending: PendingCatalog = serde_json::from_slice(bytes)?;
    if pending.version != 1
        || pending.device.is_empty()
        || pending.device.len() > 256
        || pending.device.chars().any(char::is_control)
    {
        bail!("invalid pending catalog");
    }
    pending.catalog.validate()?;
    Ok(pending)
}

fn clear_pending(root: &Path) -> anyhow::Result<()> {
    let path = root.join(PENDING_CATALOG);
    storage::remove_file(&storage::backup_path(&path))?;
    storage::remove_file(&path)
}
fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
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
fn parse(bytes: &[u8]) -> anyhow::Result<Intent> {
    if bytes.len() > 24 * 1024 * 1024 {
        bail!("structural intent is too large");
    }
    let intent: Intent = serde_json::from_slice(bytes)?;
    let resolved = intent.catalog.resolve()?;
    if intent.version != 1
        || intent.device.is_empty()
        || intent.device.len() > 256
        || intent.device.chars().any(char::is_control)
    {
        bail!("invalid structural intent");
    }
    let mut ids = BTreeSet::new();
    for item in &intent.items {
        let entry = resolved
            .get(&item.id)
            .context("unknown structural identity")?;
        if !ids.insert(&item.id)
            || !valid_path(&item.old)
            || item.is_dir != entry.is_dir
            || item.restore_from.as_ref().is_some_and(|id| !valid_id(id))
            || match &item.new {
                Some(path) => !valid_path(path) || entry.deleted() || path != &entry.path,
                None => !entry.deleted(),
            }
        {
            bail!("invalid structural item");
        }
    }
    if intent
        .before
        .iter()
        .any(|(id, path)| !resolved.contains_key(id) || !valid_path(path))
    {
        bail!("invalid structural bindings");
    }
    Ok(intent)
}
fn read_intent(directory: &Path) -> anyhow::Result<Option<Intent>> {
    storage::read_validated(&directory.join("intent.json"), |bytes| parse(bytes).is_ok())?
        .map(|bytes| parse(&bytes))
        .transpose()
}
fn saved_state(directory: &Path, id: &str) -> PathBuf {
    directory.join("states").join(format!("{id}.bin"))
}
fn decode_doc(path: &str, bytes: &[u8]) -> anyhow::Result<Doc> {
    let (_, update) = CrdtManager::decode_file(&CrdtManager::state_relative_path(path), bytes)?;
    let doc = Doc::new();
    doc.transact_mut()
        .apply_update(Update::decode_v1(update)?)?;
    Ok(doc)
}
fn doc_text(doc: &Doc) -> String {
    doc.get_or_insert_text("content")
        .get_string(&doc.transact())
}
pub fn register_generated(root: &Path, path: &str) -> anyhow::Result<()> {
    if catalog::file_path(root).exists() {
        catalog::transact(root, |catalog| {
            catalog.seed_generated(path)?;
            Ok(())
        })?;
    }
    Ok(())
}
fn write_progress(directory: &Path, progress: &Progress) -> anyhow::Result<()> {
    storage::write_validated(
        &directory.join("progress.json"),
        &serde_json::to_vec(progress)?,
        |bytes| serde_json::from_slice::<Progress>(bytes).is_ok(),
    )
}

/// A creation deleted before it ever reached the filesystem still has a full
/// restore payload. Use the ordinary trash format, so restart/undo and future
/// trash selection use the same validated archive as other deletions.
pub(crate) fn archive_creation(root: &Path, entry: &ResolvedEntry, content: Option<&str>, state: Option<&[u8]>, catalog: &Catalog) -> anyhow::Result<()> {
    if !entry.deleted() || entry.is_dir != content.is_none() { bail!("invalid deleted creation archive"); }
    let record = blake3::hash(format!("interrupted-creation:{}", entry.id).as_bytes()).to_hex().to_string();
    let directory = root.join(TRASH).join(record);
    fs::create_dir_all(directory.join("items"))?;
    fs::create_dir_all(directory.join("states"))?;
    let created_ms = read_intent(&directory)?.map(|intent| intent.created_ms).unwrap_or_else(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
    });
    let intent = Intent { version: 1, created_ms, device: "recovery".into(), catalog: catalog.clone(),
        before: BTreeMap::from([(entry.id.clone(), entry.path.clone())]),
        items: vec![Item { id: entry.id.clone(), old: entry.path.clone(), new: None, is_dir: entry.is_dir, location: entry.location.clone(), restore_from: None }] };
    storage::write_validated(&directory.join("intent.json"), &serde_json::to_vec(&intent)?, |bytes| parse(bytes).is_ok())?;
    if let (Some(content), Some(state)) = (content, state) {
        let target = directory.join("items").join(&entry.id);
        if target.exists() && fs::read_to_string(&target)? != content { bail!("creation archive already has different text"); }
        storage::write_text(&target, content)?;
        let mut bytes = Vec::with_capacity(4 + entry.path.len() + state.len());
        bytes.extend_from_slice(&(entry.path.len() as u32).to_be_bytes());
        bytes.extend_from_slice(entry.path.as_bytes());
        bytes.extend_from_slice(state);
        storage::write_validated(&saved_state(&directory, &entry.id), &bytes, |bytes| decode_doc(&entry.path, bytes).is_ok())?;
    } else if entry.is_dir {
        fs::create_dir_all(directory.join("items").join(&entry.id))?;
    } else { bail!("creation archive has no collaborative state"); }
    Ok(())
}

/// Copies are deterministic across peers and never live inside a deleted folder.
pub fn preserve_deleted(
    root: &Path,
    entry: &ResolvedEntry,
    content: &str,
    state: Option<&[u8]>,
) -> anyhow::Result<Option<String>> {
    let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
    if !entry.deleted()
        || entry
            .deletions
            .values()
            .all(|expected| expected.as_ref() == Some(&hash))
    {
        return Ok(None);
    }
    let stem = Path::new(&entry.path)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("note");
    let mut stem = stem.to_string();
    while stem.len() > 170 {
        stem.pop();
    }
    let copy = format!(
        "{stem} (deleted conflict {}-{}).md",
        &entry.id[..8],
        &hash[..10]
    );
    let target = vault::safe_join(root, &copy)?;
    if target.exists() {
        if fs::read_to_string(&target)? != content {
            bail!("conflict copy hash collision");
        }
        return Ok(None);
    }
    if let Some(update) = state {
        crate::note_transaction::commit(root, &copy, content, update)?;
    } else {
        storage::write_text(&target, content)?;
    }
    register_generated(root, &copy)?;
    storage::report_recovery(&target, true);
    Ok(Some(copy))
}

pub(crate) fn move_preserving(source: &Path, destination: &Path) -> anyhow::Result<()> {
    if !destination.exists() {
        fs::create_dir_all(destination.parent().context("missing destination parent")?)?;
        fs::rename(source, destination).with_context(|| {
            format!(
                "place structural item {} -> {}",
                source.display(),
                destination.display()
            )
        })?;
        return Ok(());
    }
    if source.is_dir() && destination.is_dir() {
        for child in fs::read_dir(source)? {
            let child = child?;
            if child.file_type()?.is_symlink() {
                bail!("cannot merge a symbolic link into an occupied destination");
            }
            move_preserving(&child.path(), &destination.join(child.file_name()))?;
        }
        fs::remove_dir(source)?;
    } else if source.is_file() && destination.is_file() {
        // Preserve an external file that appeared after staging. Never replace
        // it before its copy has been committed to disk.
        let previous = fs::read(destination)?;
        let hash = blake3::hash(&previous).to_hex().to_string();
        let name = destination
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("file");
        let extension = destination
            .extension()
            .and_then(|name| name.to_str())
            .unwrap_or("bin");
        let copy = destination.with_file_name(format!(
            "{name} (move conflict {}).{extension}",
            &hash[..10]
        ));
        if copy.exists() && fs::read(&copy)? != previous {
            bail!("move conflict copy collision");
        }
        storage::write_validated(&copy, &previous, |bytes| bytes == previous.as_slice())?;
        storage::report_recovery(&copy, true);
        storage::remove_file(destination)?;
        fs::rename(source, destination)?;
    } else {
        bail!("structural destination has a different type");
    }
    Ok(())
}
fn copy_tree(source: &Path, destination: &Path) -> anyhow::Result<()> {
    let metadata = source.symlink_metadata()?;
    if metadata.is_symlink() {
        bail!("errors.undoUnsupportedItem");
    }
    if metadata.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else {
        let bytes = fs::read(source)?;
        storage::write_validated(destination, &bytes, |data| data == bytes.as_slice())
            .with_context(|| {
                format!(
                    "copy archived file {} -> {}",
                    source.display(),
                    destination.display()
                )
            })?;
    }
    Ok(())
}

pub(crate) fn preserve_recreated_source(
    root: &Path,
    directory: &Path,
    id: &str,
    source: &Path,
) -> anyhow::Result<()> {
    for entry in walkdir::WalkDir::new(source).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() || !vault::is_markdown(entry.path()) {
            continue;
        }
        let content = fs::read_to_string(entry.path())?;
        let original_path = entry.path().strip_prefix(root)?.to_string_lossy();
        let mut stem = entry
            .path()
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("note")
            .to_string();
        while stem.len() > 160 {
            stem.pop();
        }
        let hash = blake3::hash(format!("{id}:{original_path}:{content}").as_bytes())
            .to_hex()
            .to_string();
        let copy = format!("{stem} (recreated conflict {}).md", &hash[..12]);
        let destination = vault::safe_join(root, &copy)?;
        if destination.exists() && fs::read_to_string(&destination)? != content {
            bail!("recreated conflict copy collision");
        }
        storage::write_text(&destination, &content)?;
        register_generated(root, &copy)?;
        storage::report_recovery(&destination, true);
    }
    let archived = directory
        .join("external")
        .join(id)
        .join(catalog::random_id());
    fs::create_dir_all(archived.parent().unwrap())?;
    fs::rename(source, archived)?;
    Ok(())
}

fn finish(
    root: &Path,
    directory: &Path,
    intent: &Intent,
    manager: &CrdtManager,
    recovered: bool,
    hook: &impl Fn(u8) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    catalog::transact(root, |current| {
        *current = current.merged(&intent.catalog)?;
        Ok(())
    })?;
    let mut progress: Progress =
        match storage::read_validated(&directory.join("progress.json"), |bytes| {
            serde_json::from_slice::<Progress>(bytes).is_ok()
        })? {
            Some(bytes) => serde_json::from_slice(&bytes)?,
            None => Progress::default(),
        };
    if (!progress.staged && !progress.placed.is_empty())
        || progress
            .placed
            .iter()
            .any(|id| !intent.items.iter().any(|item| &item.id == id))
    {
        bail!("invalid structural progress");
    }
    if !progress.staged {
        let mut ordered: Vec<_> = intent.items.iter().collect();
        ordered.sort_by_key(|item| {
            (
                item.is_dir,
                std::cmp::Reverse(item.old.matches('/').count()),
            )
        });
        for item in ordered {
            let staged = directory.join("items").join(&item.id);
            let source = if let Some(record) = &item.restore_from {
                root.join(TRASH).join(record).join("items").join(&item.id)
            } else {
                vault::safe_join(root, &item.old)?
            };
            if staged.exists() {
                if item.restore_from.is_none() && source.exists() {
                    preserve_recreated_source(root, directory, &item.id, &source)?;
                }
                continue;
            }
            if !source.exists() {
                bail!("structural source is unavailable");
            }
            fs::create_dir_all(staged.parent().unwrap())?;
            if item.restore_from.is_some() {
                copy_tree(&source, &staged)?;
            } else {
                fs::rename(&source, &staged).with_context(|| {
                    format!(
                        "stage structural source {} -> {}",
                        source.display(),
                        staged.display()
                    )
                })?;
            }
            hook(1)?;
        }
        progress.staged = true;
        write_progress(directory, &progress)?;
        hook(2)?;
    }
    let resolved = intent.catalog.resolve()?;
    let mut ordered: Vec<_> = intent.items.iter().collect();
    ordered.sort_by_key(|item| {
        (
            !item.is_dir,
            item.new
                .as_ref()
                .map_or(0, |path| path.matches('/').count()),
        )
    });
    for item in ordered {
        if progress.placed.contains(&item.id) {
            let target = item
                .new
                .as_ref()
                .map(|path| vault::safe_join(root, path))
                .transpose()?
                .unwrap_or_else(|| directory.join("items").join(&item.id));
            if !target.exists() {
                bail!("completed structural item is unavailable");
            }
            continue;
        }
        let staged = directory.join("items").join(&item.id);
        if let Some(new) = &item.new {
            let destination = vault::safe_join(root, new)?;
            if staged.exists() {
                move_preserving(&staged, &destination)?;
            }
            if !destination.exists() {
                bail!("structural destination is unavailable");
            }
            if !item.is_dir {
                let bytes = storage::read_validated(&saved_state(directory, &item.id), |bytes| {
                    decode_doc(&item.old, bytes).is_ok()
                })?
                .context("structural history is unavailable")?;
                let doc = decode_doc(&item.old, &bytes)?;
                let content = fs::read_to_string(&destination)?;
                if let Some(current) =
                    CrdtManager::read_state_file(root, &CrdtManager::state_relative_path(new))?
                {
                    // The path can have belonged to another identity before a
                    // swap. Only merge a partial replay for this same identity.
                    if intent.before.get(&item.id) == Some(new)
                        || !intent.before.values().any(|old| old == new)
                    {
                        let other = decode_doc(new, &current)?;
                        doc.transact_mut()
                            .apply_update(Update::decode_v1(&CrdtManager::encode_state(&other))?)?;
                    }
                }
                CrdtManager::replace_text(&doc, &content);
                crate::note_transaction::commit(
                    root,
                    new,
                    &content,
                    &CrdtManager::encode_state(&doc),
                )?;
            }
        } else if !item.is_dir {
            let content = fs::read_to_string(&staged)?;
            let bytes = storage::read_validated(&saved_state(directory, &item.id), |bytes| {
                decode_doc(&item.old, bytes).is_ok()
            })?
            .context("deleted history is unavailable")?;
            let doc = decode_doc(&item.old, &bytes)?;
            CrdtManager::replace_text(&doc, &content);
            preserve_deleted(
                root,
                &resolved[&item.id],
                &content,
                Some(&CrdtManager::encode_state(&doc)),
            )?;
        }
        progress.placed.insert(item.id.clone());
        write_progress(directory, &progress)?;
        hook(3)?;
    }
    let mut bindings = structural::load_paths(root)?;
    let active_names: BTreeSet<_> = resolved
        .values()
        .filter(|entry| !entry.deleted())
        .map(|entry| entry.path.as_str())
        .collect();
    for item in &intent.items {
        if item.restore_from.is_none() && !active_names.contains(item.old.as_str()) {
            let old = vault::safe_join(root, &item.old)?;
            if old.exists() {
                preserve_recreated_source(root, directory, &item.id, &old)?;
            }
        }
    }
    let live_paths: BTreeSet<_> = resolved
        .values()
        .filter(|entry| !entry.deleted() && !entry.is_dir)
        .map(|entry| entry.path.as_str())
        .collect();
    for (id, entry) in &resolved {
        if entry.deleted() {
            bindings.paths.remove(id);
        } else {
            let target = vault::safe_join(root, &entry.path)?;
            if entry.is_dir {
                fs::create_dir_all(&target)?;
            }
            if target.exists() {
                bindings.paths.insert(id.clone(), entry.path.clone());
            }
        }
    }
    structural::save_paths(root, &bindings)?;
    for item in &intent.items {
        if !item.is_dir && !live_paths.contains(item.old.as_str()) {
            let file = root.join(CrdtManager::state_relative_path(&item.old));
            storage::remove_file(&storage::backup_path(&file))?;
            storage::remove_file(&file)?;
        }
        manager.invalidate_path(root, &item.old)?;
        if let Some(new) = &item.new {
            manager.invalidate_path(root, new)?;
        }
    }
    crate::reference_sync::normalize_all(root, manager, &catalog::load(root)?)?;
    catalog::transact(root, |current| {
        current.acknowledge(&intent.device, intent.catalog.operations.keys().cloned())
    })?;
    hook(4)?;
    // Retain the original CRDT states and removed files. The archive also
    // records their identities and locations for a later observed restore.
    fs::create_dir_all(root.join(TRASH))?;
    fs::rename(
        directory,
        root.join(TRASH).join(directory.file_name().unwrap()),
    )
    .context("archive completed structural transaction")?;
    if recovered {
        storage::report_recovery(&root.join(catalog::RELATIVE_PATH), true);
    }
    Ok(())
}
fn recover_inner(root: &Path, manager: &CrdtManager) -> anyhow::Result<()> {
    let pending = root.join(PENDING);
    if pending.exists() {
        for entry in fs::read_dir(&pending)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() || !valid_id(&entry.file_name().to_string_lossy()) {
                continue;
            }
            let Some(intent) = read_intent(&entry.path())? else {
                let contains_data = ["items", "external"].iter().any(|name| {
                    fs::read_dir(entry.path().join(name))
                        .is_ok_and(|mut entries| entries.next().is_some())
                });
                if contains_data {
                    storage::report_recovery(&entry.path(), false);
                    bail!("structural recovery data has no valid intent");
                }
                continue;
            };
            if let Err(error) = finish(root, &entry.path(), &intent, manager, true, &|_| Ok(())) {
                storage::report_recovery(&entry.path().join("intent.json"), false);
                return Err(error);
            }
        }
    }
    let queued = root.join(PENDING_CATALOG);
    if let Some(bytes) = storage::read_validated(&queued, |bytes| parse_pending(bytes).is_ok())? {
        let pending = parse_pending(&bytes)?;
        if let Err(error) =
            materialize(
                root,
                manager,
                &pending.catalog,
                &pending.device,
                &|_| Ok(()),
            )
        {
            storage::report_recovery(&queued, false);
            return Err(error);
        }
        clear_pending(root)?;
        storage::report_recovery(&queued, true);
    }
    Ok(())
}
pub fn recover_all(root: &Path, manager: &CrdtManager) -> anyhow::Result<()> {
    if BUSY.get() {
        return Ok(());
    }
    structural::exclusive(root, manager, || {
        let _scope = Scope::enter();
        recover_inner(root, manager)
    })
}

pub fn prepare(root: &Path, manager: &CrdtManager, device: &str) -> anyhow::Result<Catalog> {
    structural::exclusive(root, manager, || {
        let _scope = Scope::enter();
        recover_inner(root, manager)?;
        let catalog = catalog::transact(root, |catalog| {
            catalog.discover_existing(root, device)?;
            Ok(catalog.clone())
        })?;
        let mut bindings = structural::load_paths(root)?;
        let mut owned: BTreeSet<_> = bindings.paths.values().cloned().collect();
        for (id, entry) in catalog.resolve()? {
            if entry.deleted() {
                continue;
            }
            if bindings.paths.contains_key(&id) {
                continue;
            }
            let candidate = std::iter::once(&entry.path)
                .chain(entry.aliases.iter())
                .find(|path| !owned.contains(*path) && root.join(path).exists());
            if let Some(path) = candidate {
                bindings.paths.insert(id, path.clone());
                owned.insert(path.clone());
            }
        }
        structural::save_paths(root, &bindings)?;
        crate::links::bind_identities(root, &catalog)?;
        crate::reference_sync::bind_all(root, manager, &catalog, &bindings.paths)?;
        Ok(catalog)
    })
}
fn restore_source(root: &Path, id: &str) -> anyhow::Result<Option<(String, Item)>> {
    let directory = root.join(TRASH);
    if !directory.exists() {
        return Ok(None);
    }
    let mut latest = None;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() || !valid_id(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let Some(intent) = read_intent(&entry.path())? else {
            continue;
        };
        let Some(item) = intent
            .items
            .iter()
            .find(|item| item.id == id && item.new.is_none())
        else {
            continue;
        };
        if !entry.path().join("items").join(id).exists() {
            continue;
        }
        let order = (
            intent.created_ms,
            intent
                .catalog
                .operations
                .values()
                .map(|op| op.clock)
                .max()
                .unwrap_or(0),
            entry.file_name().to_string_lossy().into_owned(),
        );
        if latest.as_ref().is_none_or(|(time, _, _)| *time < order) {
            latest = Some((
                order,
                entry.file_name().to_string_lossy().into_owned(),
                item.clone(),
            ));
        }
    }
    Ok(latest.map(|(_, record, item)| (record, item)))
}
pub fn deleted_state(root: &Path, id: &str) -> anyhow::Result<Option<Vec<u8>>> {
    let Some((record, item)) = restore_source(root, id)? else {
        return Ok(None);
    };
    let bytes =
        storage::read_validated(&saved_state(&root.join(TRASH).join(record), id), |bytes| {
            decode_doc(&item.old, bytes).is_ok()
        })?
        .context("deleted history is unavailable")?;
    Ok(Some(CrdtManager::encode_state(&decode_doc(
        &item.old, &bytes,
    )?)))
}
pub fn merge(
    root: &Path,
    manager: &CrdtManager,
    remote: &Catalog,
    device: &str,
) -> anyhow::Result<usize> {
    merge_with_hook(root, manager, remote, device, |_| Ok(()))
}
fn merge_with_hook(
    root: &Path,
    manager: &CrdtManager,
    remote: &Catalog,
    device: &str,
    hook: impl Fn(u8) -> anyhow::Result<()>,
) -> anyhow::Result<usize> {
    prepare(root, manager, device)?;
    structural::exclusive(root, manager, || {
        let _scope = Scope::enter();
        recover_inner(root, manager)?;
        let planned = catalog::load(root)?.merged(remote)?;
        let queued = PendingCatalog {
            version: 1,
            device: device.into(),
            catalog: planned,
        };
        storage::write_validated(
            &root.join(PENDING_CATALOG),
            &serde_json::to_vec(&queued)?,
            |bytes| parse_pending(bytes).is_ok(),
        )?;
        hook(6)?;
        let count = materialize(root, manager, &queued.catalog, device, &hook)?;
        clear_pending(root)?;
        Ok(count)
    })
}

fn materialize(
    root: &Path,
    manager: &CrdtManager,
    remote: &Catalog,
    device: &str,
    hook: &impl Fn(u8) -> anyhow::Result<()>,
) -> anyhow::Result<usize> {
    let planned = catalog::transact(root, |current| {
        *current = current.merged(remote)?;
        Ok(current.clone())
    })?;
    hook(5)?;
    let bindings = structural::load_paths(root)?;
    let mut items = Vec::new();
    for (id, entry) in planned.resolve()? {
        if let Some(old) = bindings.paths.get(&id) {
            if !entry.deleted() && old == &entry.path {
                continue;
            }
            if !root.join(old).exists() {
                continue;
            }
            items.push(Item {
                id,
                old: old.clone(),
                new: (!entry.deleted()).then_some(entry.path),
                is_dir: entry.is_dir,
                location: entry.location,
                restore_from: None,
            });
        } else if !entry.deleted() {
            if let Some((record, original)) = restore_source(root, &id)? {
                items.push(Item {
                    id,
                    old: original.old,
                    new: Some(entry.path),
                    is_dir: entry.is_dir,
                    location: original.location,
                    restore_from: Some(record),
                });
            }
        }
    }
    if items.is_empty() {
        let mut bindings = bindings;
        for (id, entry) in planned.resolve()? {
            if entry.deleted() || !entry.is_dir {
                continue;
            }
            fs::create_dir_all(vault::safe_join(root, &entry.path)?)?;
            bindings.paths.insert(id, entry.path);
        }
        structural::save_paths(root, &bindings)?;
        crate::reference_sync::normalize_all(root, manager, &planned)?;
        catalog::transact(root, |current| {
            current.acknowledge(device, planned.operations.keys().cloned())
        })?;
        return Ok(0);
    }
    let directory = root.join(PENDING).join(catalog::random_id());
    fs::create_dir_all(directory.join("states"))?;
    for item in &items {
        if item.is_dir {
            continue;
        }
        let bytes = if let Some(record) = &item.restore_from {
            storage::read_validated(
                &saved_state(&root.join(TRASH).join(record), &item.id),
                |bytes| decode_doc(&item.old, bytes).is_ok(),
            )?
            .context("restored history is unavailable")?
        } else {
            manager.get_or_create_doc(root, &item.old)?;
            CrdtManager::read_state_file(root, &CrdtManager::state_relative_path(&item.old))?
                .context("structural history is unavailable")?
        };
        storage::write_validated(&saved_state(&directory, &item.id), &bytes, |bytes| {
            decode_doc(&item.old, bytes).is_ok()
        })?;
    }
    let count = items.len();
    let intent = Intent {
        version: 1,
        created_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64,
        device: device.into(),
        catalog: planned,
        before: bindings.paths,
        items,
    };
    storage::write_validated(
        &directory.join("intent.json"),
        &serde_json::to_vec_pretty(&intent)?,
        |bytes| parse(bytes).is_ok(),
    )?;
    hook(0)?;
    finish(root, &directory, &intent, manager, false, hook)?;
    Ok(count)
}
pub fn delete(root: &Path, path: &str, manager: &CrdtManager, device: &str) -> anyhow::Result<()> {
    delete_with_hashes(root, path, manager, device, true)
}

pub(crate) fn delete_legacy(
    root: &Path,
    path: &str,
    manager: &CrdtManager,
    device: &str,
) -> anyhow::Result<bool> {
    let catalog = prepare(root, manager, device)?;
    let resolved = catalog.resolve()?;
    if resolved
        .values()
        .any(|entry| entry.deleted() && entry.aliases.contains(path))
    {
        return Ok(false);
    }
    let Some(entry) = resolved
        .values()
        .find(|entry| !entry.deleted() && entry.aliases.contains(path))
    else {
        return Ok(false);
    };
    delete_with_hashes(root, &entry.path, manager, device, false)?;
    Ok(true)
}

fn delete_with_hashes(
    root: &Path,
    path: &str,
    manager: &CrdtManager,
    device: &str,
    known_hash: bool,
) -> anyhow::Result<()> {
    let catalog = prepare(root, manager, device)?;
    let resolved = catalog.resolve()?;
    let target = resolved
        .values()
        .find(|entry| !entry.deleted() && entry.path == path)
        .context("errors.sourceNotFound")?;
    let observed = resolved
        .values()
        .filter(|entry| {
            !entry.deleted() && (entry.path == path || entry.path.starts_with(&format!("{path}/")))
        })
        .map(|entry| {
            Ok((
                entry.id.clone(),
                if entry.is_dir || !known_hash {
                    None
                } else {
                    Some(
                        blake3::hash(fs::read(vault::safe_join(root, &entry.path)?)?.as_slice())
                            .to_hex()
                            .to_string(),
                    )
                },
            ))
        })
        .collect::<anyhow::Result<BTreeMap<_, _>>>()?;
    let planned = catalog::transact(root, |current| {
        current.push(
            device,
            Change::Delete {
                id: target.id.clone(),
                observed,
            },
        )?;
        Ok(current.clone())
    })?;
    merge(root, manager, &planned, device)?;
    Ok(())
}
#[derive(Clone, Debug, Serialize)]
pub struct TrashEntry {
    pub record_id: String,
    pub note_id: String,
    pub path: String,
    pub is_dir: bool,
    pub deleted_ms: u64,
    pub items: usize,
    #[serde(skip)]
    clock: u64,
}

/// Only the most recent archive of each still-deleted identity is offered.
/// A directory is one selection, with its archived descendants restored together.
pub fn list_trash(root: &Path) -> anyhow::Result<Vec<TrashEntry>> {
    let resolved = catalog::load(root)?.resolve()?;
    let directory = root.join(TRASH);
    if !directory.exists() { return Ok(Vec::new()); }
    let mut latest: BTreeMap<String, TrashEntry> = BTreeMap::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let record = entry.file_name().to_string_lossy().into_owned();
        if !entry.file_type()?.is_dir() || !valid_id(&record) { continue; }
        let Some(intent) = read_intent(&entry.path())? else { continue; };
        let clock = intent.catalog.operations.values().map(|op| op.clock).max().unwrap_or(0);
        for item in intent.items {
            if item.new.is_some() || !resolved.get(&item.id).is_some_and(ResolvedEntry::deleted)
                || !entry.path().join("items").join(&item.id).exists() { continue; }
            let candidate = TrashEntry { record_id: record.clone(), note_id: item.id.clone(),
                path: item.old, is_dir: item.is_dir, deleted_ms: intent.created_ms, items: 1, clock };
            if latest.get(&item.id).is_none_or(|old| (old.deleted_ms, old.clock, &old.record_id)
                < (candidate.deleted_ms, candidate.clock, &candidate.record_id)) {
                latest.insert(item.id, candidate);
            }
        }
    }
    let mut entries: Vec<_> = latest.values().filter(|item| !latest.values().any(|parent|
        parent.is_dir && parent.record_id == item.record_id && item.path.starts_with(&format!("{}/", parent.path))))
        .cloned().collect();
    for entry in &mut entries {
        entry.items = latest.values().filter(|item| item.record_id == entry.record_id
            && (item.note_id == entry.note_id || (entry.is_dir && item.path.starts_with(&format!("{}/", entry.path))))).count();
    }
    entries.sort_by(|a, b| (b.deleted_ms, b.clock, &b.record_id, &b.path).cmp(&(a.deleted_ms, a.clock, &a.record_id, &a.path)));
    Ok(entries)
}

pub fn read_trash_note(root: &Path, record: &str, id: &str) -> anyhow::Result<String> {
    if !valid_id(record) || !valid_id(id) { bail!("invalid trash selection"); }
    let intent = read_intent(&root.join(TRASH).join(record))?.context("history.unavailable")?;
    let item = intent.items.iter().find(|item| item.id == id && item.new.is_none() && !item.is_dir)
        .context("history.unavailable")?;
    let file = root.join(TRASH).join(record).join("items").join(id);
    if fs::metadata(&file)?.len() > vault::MAX_NOTE_BYTES { bail!("errors.noteTooLarge"); }
    let content = fs::read_to_string(file)?;
    let state = storage::read_validated(&saved_state(&root.join(TRASH).join(record), id),
        |bytes| decode_doc(&item.old, bytes).is_ok())?.context("deleted history is unavailable")?;
    if doc_text(&decode_doc(&item.old, &state)?) != content { bail!("trash text and collaborative history differ"); }
    Ok(content)
}

pub fn restore_latest(
    root: &Path,
    manager: &CrdtManager,
    device: &str,
) -> anyhow::Result<Option<(String, bool)>> {
    structural::exclusive(root, manager, || {
        prepare(root, manager, device)?;
        let Some(entry) = list_trash(root)?.into_iter().next() else { return Ok(None); };
        restore_selected(root, &entry.record_id, &entry.note_id, manager, device).map(Some)
    })
}

pub fn restore_selected(root: &Path, record: &str, id: &str, manager: &CrdtManager, device: &str) -> anyhow::Result<(String, bool)> {
    if !valid_id(record) || !valid_id(id) { bail!("invalid trash selection"); }
    structural::exclusive(root, manager, || {
    prepare(root, manager, device)?;
    let resolved = catalog::load(root)?.resolve()?;
    let directory = root.join(TRASH).join(record);
    let intent = read_intent(&directory)?.context("history.unavailable")?;
    let target = intent.items.iter().find(|item| item.id == id && item.new.is_none()
        && resolved.get(id).is_some_and(ResolvedEntry::deleted)).context("history.unavailable")?;
    if restore_source(root, id)?.is_none_or(|(latest, _)| latest != record) { bail!("history.unavailable"); }
    let mut items: Vec<_> = intent.items.iter().filter(|item| item.new.is_none()
        && resolved.get(&item.id).is_some_and(ResolvedEntry::deleted)
        && (item.id == id || (target.is_dir && item.old.starts_with(&format!("{}/", target.old)))))
        .cloned().collect();
    // Restore any deleted ancestors too, without restoring unrelated siblings.
    let mut parent = target.location.parent.clone();
    while let Some(parent_id) = parent {
        let ancestor = resolved.get(&parent_id).context("history.unavailable")?;
        parent = ancestor.location.parent.clone();
        if ancestor.deleted() && !items.iter().any(|item| item.id == parent_id) {
            let (_, original) = restore_source(root, &parent_id)?.context("history.unavailable")?;
            items.push(original);
        }
    }
    // Validate every retained Markdown/state pair before publishing a restore.
    for item in &items {
        let (record, _) = restore_source(root, &item.id)?.context("history.unavailable")?;
        if !item.is_dir { read_trash_note(root, &record, &item.id)?; }
    }
    items.sort_by_key(|item| {
        (
            item.old.matches('/').count(),
            !item.is_dir,
            item.old.clone(),
        )
    });
    let root_id = id.to_string();
    let is_dir = target.is_dir;
    let mut occupied: BTreeSet<_> = resolved
        .values()
        .filter(|entry| !entry.deleted())
        .map(|entry| {
            (
                entry.location.parent.clone(),
                entry.location.name.to_lowercase(),
            )
        })
        .collect();
    let mut locations = BTreeMap::new();
    for item in items {
        let mut location = item.location;
        if occupied.contains(&(location.parent.clone(), location.name.to_lowercase())) {
            let original = location.name.clone();
            for counter in 0.. {
                location.name =
                    crate::catalog::conflict_name(&original, &item.id, item.is_dir, counter)
                        .replace(" (path conflict ", " (restored ");
                if !occupied.contains(&(location.parent.clone(), location.name.to_lowercase())) {
                    break;
                }
            }
        }
        occupied.insert((location.parent.clone(), location.name.to_lowercase()));
        locations.insert(item.id, location);
    }
    let planned = catalog::transact(root, |catalog| {
        catalog.push(device, Change::Restore { locations })?;
        Ok(catalog.clone())
    })?;
    merge(root, manager, &planned, device)?;
    Ok((
        catalog::load(root)?.resolve()?[&root_id].path.clone(),
        is_dir,
    ))
    })
}

pub fn has_restorable(root: &Path) -> anyhow::Result<bool> {
    let resolved = catalog::load(root)?.resolve()?;
    for (id, entry) in resolved {
        if entry.deleted() && restore_source(root, &id)?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn expire_trash(root: &Path, cutoff: Option<u64>, peers: &[String], apply: bool, report: &mut crate::retention::CleanupReport) -> anyhow::Result<()> {
    let Some(cutoff) = cutoff else { return Ok(()); };
    let directory = root.join(TRASH);
    if !directory.exists() { return Ok(()); }
    let current = catalog::load(root)?;
    if !crate::retention::owned_path(root, &directory, false)? { report.protected += 1; return Ok(()); }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() || !valid_id(&entry.file_name().to_string_lossy()) { continue; }
        if !crate::retention::owned_path(root, &entry.path(), true)? { report.protected += 1; continue; }
        let Some(intent) = read_intent(&entry.path())? else { report.protected += 1; continue; };
        if intent.created_ms >= cutoff { continue; }
        // External files retained during recovery have not been reviewed here.
        if entry.path().join("external").exists() { report.protected += 1; continue; }
        let resolved = intent.catalog.resolve()?;
        let required: BTreeSet<_> = intent.items.iter().filter(|item| item.new.is_none())
            .flat_map(|item| resolved[&item.id].deletions.keys().cloned()).collect();
        if !required.is_empty() && peers.iter().any(|peer| current.acknowledgements.get(peer).is_none_or(|seen| !required.is_subset(seen))) {
            report.protected += 1; continue;
        }
        let mut complete = true;
        for item in intent.items.iter().filter(|item| item.new.is_none()) {
            let file = entry.path().join("items").join(&item.id);
            if !file.exists() { complete = false; break; }
            if !item.is_dir {
                let state = storage::read_validated(&saved_state(&entry.path(), &item.id), |bytes| decode_doc(&item.old, bytes).is_ok())?;
                if fs::metadata(&file)?.len() > vault::MAX_NOTE_BYTES
                    || state.as_ref().is_none_or(|bytes| decode_doc(&item.old, bytes).is_err()) {
                    complete = false; break;
                }
                if doc_text(&decode_doc(&item.old, &state.unwrap())?) != fs::read_to_string(file)? { complete = false; break; }
            }
        }
        if !complete { report.protected += 1; continue; }
        report.archives += 1;
        if apply { fs::remove_dir_all(entry.path())?; }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::{ReadTxn, StateVector, Text};
    #[test]
    fn selected_trash_restore_keeps_other_deletions_and_reused_filenames() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        crate::creation::create(root.path(), "a.md", Some("original a"), &manager, "a").unwrap();
        crate::creation::create(root.path(), "b.md", Some("original b"), &manager, "a").unwrap();
        delete(root.path(), "a.md", &manager, "a").unwrap();
        delete(root.path(), "b.md", &manager, "a").unwrap();
        crate::creation::create(root.path(), "a.md", Some("replacement"), &manager, "a").unwrap();
        let entries = list_trash(root.path()).unwrap();
        let selected = entries.iter().find(|entry| entry.path == "a.md").unwrap();
        assert_eq!(read_trash_note(root.path(), &selected.record_id, &selected.note_id).unwrap(), "original a");
        let (path, is_dir) = restore_selected(root.path(), &selected.record_id, &selected.note_id, &CrdtManager::new(), "a").unwrap();
        assert!(!is_dir);
        assert_ne!(path, "a.md");
        assert_eq!(vault::read_note(root.path(), &path).unwrap(), "original a");
        assert_eq!(vault::read_note(root.path(), "a.md").unwrap(), "replacement");
        assert!(!root.path().join("b.md").exists());
        let remaining = list_trash(root.path()).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].path, "b.md");
    }
    #[test]
    fn invalid_archived_history_cannot_publish_a_restore_or_discard_its_original() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        crate::creation::create(root.path(), "note.md", Some("original"), &manager, "a").unwrap();
        delete(root.path(), "note.md", &manager, "a").unwrap();
        let entry = list_trash(root.path()).unwrap().remove(0);
        let directory = root.path().join(TRASH).join(&entry.record_id);
        let state = saved_state(&directory, &entry.note_id);
        storage::remove_file(&storage::backup_path(&state)).unwrap();
        fs::write(&state, "damaged history").unwrap();
        let before = catalog::load(root.path()).unwrap();
        assert!(restore_selected(root.path(), &entry.record_id, &entry.note_id, &CrdtManager::new(), "a").is_err());
        assert_eq!(catalog::load(root.path()).unwrap().operations, before.operations);
        assert!(!root.path().join("note.md").exists());
        assert_eq!(fs::read_to_string(directory.join("items").join(&entry.note_id)).unwrap(), "original");
        assert_eq!(fs::read_to_string(state).unwrap(), "damaged history");
    }
    #[test]
    fn folder_trash_selection_restores_its_children_assets_and_history_after_restart() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        crate::creation::create(root.path(), "folder/sub/note.md", Some("before"), &manager, "a").unwrap();
        fs::write(root.path().join("folder/sub/asset.bin"), [0, 1, 255]).unwrap();
        manager.replace_note_text(root.path(), "folder/sub/note.md", "after").unwrap();
        let id = catalog::load(root.path()).unwrap().resolve().unwrap().into_values().find(|entry| entry.path == "folder/sub/note.md").unwrap().id;
        delete(root.path(), "folder", &manager, "a").unwrap();
        let entries = list_trash(root.path()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].items, 3);
        let manager = CrdtManager::new();
        let restored = restore_selected(root.path(), &entries[0].record_id, &entries[0].note_id, &manager, "a").unwrap();
        assert_eq!(restored, ("folder".into(), true));
        assert_eq!(fs::read(root.path().join("folder/sub/asset.bin")).unwrap(), [0, 1, 255]);
        assert_eq!(vault::read_note(root.path(), "folder/sub/note.md").unwrap(), "after");
        assert_eq!(crate::note_history::list(root.path(), &id).unwrap().len(), 1);
        assert!(list_trash(root.path()).unwrap().is_empty());
    }
    #[test]
    fn empty_remote_directories_are_materialized_and_keep_identity_through_a_move() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        vault::create_folder(a.path(), "empty/nested").unwrap();
        let snapshot = prepare(a.path(), &manager, "a").unwrap();
        merge(b.path(), &manager, &snapshot, "b").unwrap();
        assert!(b.path().join("empty/nested").is_dir());
        crate::structural::rename(a.path(), "empty", "moved", &manager, "a").unwrap();
        merge(b.path(), &manager, &catalog::load(a.path()).unwrap(), "b").unwrap();
        assert!(b.path().join("moved/nested").is_dir());
        assert!(!b.path().join("empty").exists());
    }
    #[test]
    fn deletion_survives_restart_preserves_an_offline_edit_and_restores_history() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        for root in [a.path(), b.path()] {
            fs::create_dir_all(root.join("folder/sub")).unwrap();
            fs::write(root.join("folder/sub/note.md"), "baseline\n").unwrap();
            fs::write(root.join("folder/sub/asset.bin"), [0, 1, 255]).unwrap();
        }
        let manager = CrdtManager::new();
        prepare(a.path(), &manager, "a").unwrap();
        prepare(b.path(), &manager, "b").unwrap();
        let before = manager
            .get_or_create_doc(b.path(), "folder/sub/note.md")
            .unwrap();
        let offline = Doc::new();
        offline
            .transact_mut()
            .apply_update(Update::decode_v1(&before).unwrap())
            .unwrap();
        offline
            .get_or_insert_text("content")
            .push(&mut offline.transact_mut(), "offline edit\n");
        manager
            .apply_update(
                b.path(),
                "folder/sub/note.md",
                &offline.transact().encode_diff_v1(&StateVector::default()),
            )
            .unwrap();
        delete(a.path(), "folder", &manager, "a").unwrap();
        let deleted = catalog::load(a.path()).unwrap();
        merge(b.path(), &CrdtManager::new(), &deleted, "b").unwrap();
        assert!(!b.path().join("folder").exists());
        let copies: Vec<_> = fs::read_dir(b.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("deleted conflict")
            })
            .collect();
        assert_eq!(copies.len(), 1);
        assert_eq!(
            fs::read_to_string(copies[0].path()).unwrap(),
            "baseline\noffline edit\n"
        );
        merge(b.path(), &CrdtManager::new(), &deleted, "b").unwrap();
        assert!(!b.path().join("folder").exists());
        let restored = restore_latest(b.path(), &CrdtManager::new(), "b")
            .unwrap()
            .unwrap();
        assert_eq!(restored, ("folder".into(), true));
        assert_eq!(
            fs::read_to_string(b.path().join("folder/sub/note.md")).unwrap(),
            "baseline\noffline edit\n"
        );
        assert_eq!(
            fs::read(b.path().join("folder/sub/asset.bin")).unwrap(),
            [0, 1, 255]
        );
        let state = CrdtManager::new()
            .get_or_create_doc(b.path(), "folder/sub/note.md")
            .unwrap();
        let restored_doc = Doc::new();
        restored_doc
            .transact_mut()
            .apply_update(Update::decode_v1(&state).unwrap())
            .unwrap();
        restored_doc
            .transact_mut()
            .apply_update(Update::decode_v1(&before).unwrap())
            .unwrap();
        assert_eq!(doc_text(&restored_doc), "baseline\noffline edit\n");
    }
    #[test]
    fn simultaneous_moves_stage_all_sources_and_recover_after_every_phase() {
        for phase in 0..=6 {
            let root = tempfile::tempdir().unwrap();
            let manager = CrdtManager::new();
            fs::write(root.path().join("a.md"), "A\n").unwrap();
            fs::write(root.path().join("b.md"), "B\n").unwrap();
            let baseline = prepare(root.path(), &manager, "local").unwrap();
            let entries = baseline.resolve().unwrap();
            let a = entries
                .values()
                .find(|entry| entry.path == "a.md")
                .unwrap()
                .id
                .clone();
            let b = entries
                .values()
                .find(|entry| entry.path == "b.md")
                .unwrap()
                .id
                .clone();
            let mut left = baseline.clone();
            let mut right = baseline.clone();
            left.push(
                "left",
                Change::Move {
                    id: a.clone(),
                    location: Location {
                        parent: None,
                        name: "b.md".into(),
                    },
                },
            )
            .unwrap();
            right
                .push(
                    "right",
                    Change::Move {
                        id: b.clone(),
                        location: Location {
                            parent: None,
                            name: "a.md".into(),
                        },
                    },
                )
                .unwrap();
            let merged = left.merged(&right).unwrap();
            assert!(
                merge_with_hook(root.path(), &manager, &merged, "local", |step| {
                    if step == phase {
                        bail!("injected failure")
                    } else {
                        Ok(())
                    }
                })
                .is_err()
            );
            recover_all(root.path(), &CrdtManager::new()).unwrap();
            assert_eq!(fs::read_to_string(root.path().join("a.md")).unwrap(), "B\n");
            assert_eq!(fs::read_to_string(root.path().join("b.md")).unwrap(), "A\n");
            let saved = catalog::load(root.path()).unwrap();
            assert!(saved.acknowledgements["local"]
                .is_superset(&merged.operations.keys().cloned().collect()));
            assert_eq!(
                structural::load_paths(root.path()).unwrap().paths[&a],
                "b.md"
            );
        }
    }

    #[test]
    fn actual_process_exit_recovers_all_structural_phases() {
        for phase in 0..=6 {
            let root = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "catalog_sync::tests::crash_worker",
                    "--ignored",
                    "--nocapture",
                ])
                .env("LOWNOTES_STRUCTURE_CRASH_ROOT", root.path())
                .env("LOWNOTES_STRUCTURE_CRASH_PHASE", phase.to_string())
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(86), "{output:?}");
            recover_all(root.path(), &CrdtManager::new()).unwrap();
            assert_eq!(fs::read_to_string(root.path().join("a.md")).unwrap(), "B\n");
            assert_eq!(fs::read_to_string(root.path().join("b.md")).unwrap(), "A\n");
            let manager = CrdtManager::new();
            let doc = Doc::new();
            doc.transact_mut()
                .apply_update(
                    Update::decode_v1(&manager.get_or_create_doc(root.path(), "a.md").unwrap())
                        .unwrap(),
                )
                .unwrap();
            assert_eq!(doc_text(&doc), "B\n");
            assert!(fs::read_dir(root.path().join(PENDING))
                .unwrap()
                .next()
                .is_none());
        }
    }

    #[test]
    fn a_file_recreated_after_staging_is_preserved_and_does_not_undo_the_deletion() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        fs::write(root.path().join("note.md"), "original").unwrap();
        let mut catalog = prepare(root.path(), &manager, "local").unwrap();
        let id = catalog
            .resolve()
            .unwrap()
            .values()
            .find(|entry| !entry.is_dir)
            .unwrap()
            .id
            .clone();
        catalog
            .push(
                "remote",
                Change::Delete {
                    id: id.clone(),
                    observed: BTreeMap::from([(
                        id,
                        Some(blake3::hash(b"original").to_hex().to_string()),
                    )]),
                },
            )
            .unwrap();
        assert!(
            merge_with_hook(root.path(), &manager, &catalog, "local", |phase| {
                if phase == 1 {
                    bail!("interrupted")
                } else {
                    Ok(())
                }
            })
            .is_err()
        );
        fs::write(root.path().join("note.md"), "external recreation").unwrap();
        recover_all(root.path(), &CrdtManager::new()).unwrap();
        assert!(!root.path().join("note.md").exists());
        let copies = fs::read_dir(root.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("recreated conflict")
            })
            .collect::<Vec<_>>();
        assert_eq!(copies.len(), 1);
        assert_eq!(
            fs::read_to_string(copies[0].path()).unwrap(),
            "external recreation"
        );
        assert!(has_restorable(root.path()).unwrap());
    }
    #[test]
    #[ignore = "Isolated child process for structural abrupt-exit regression"]
    fn crash_worker() {
        let root = PathBuf::from(std::env::var_os("LOWNOTES_STRUCTURE_CRASH_ROOT").unwrap());
        let phase: u8 = std::env::var("LOWNOTES_STRUCTURE_CRASH_PHASE")
            .unwrap()
            .parse()
            .unwrap();
        fs::write(root.join("a.md"), "A\n").unwrap();
        fs::write(root.join("b.md"), "B\n").unwrap();
        let manager = CrdtManager::new();
        let baseline = prepare(&root, &manager, "worker").unwrap();
        let entries = baseline.resolve().unwrap();
        let a = entries
            .values()
            .find(|entry| entry.path == "a.md")
            .unwrap()
            .id
            .clone();
        let b = entries
            .values()
            .find(|entry| entry.path == "b.md")
            .unwrap()
            .id
            .clone();
        let mut left = baseline.clone();
        let mut right = baseline;
        left.push(
            "left",
            Change::Move {
                id: a,
                location: Location {
                    parent: None,
                    name: "b.md".into(),
                },
            },
        )
        .unwrap();
        right
            .push(
                "right",
                Change::Move {
                    id: b,
                    location: Location {
                        parent: None,
                        name: "a.md".into(),
                    },
                },
            )
            .unwrap();
        merge_with_hook(
            &root,
            &manager,
            &left.merged(&right).unwrap(),
            "worker",
            |step| {
                if step == phase {
                    std::process::exit(86);
                }
                Ok(())
            },
        )
        .unwrap();
        panic!("worker did not exit at requested phase");
    }
}
