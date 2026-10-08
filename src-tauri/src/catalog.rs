//! Stable file identities and a causal history for structural vault changes.
//! Operation parents describe what a device had seen; an unrelated rename
//! cannot undo a deletion, and a restore must have observed that deletion.
use anyhow::{bail, Context};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub const RELATIVE_PATH: &str = ".lownotes/catalog.json";
const MAX_BYTES: usize = 16 * 1024 * 1024;
static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub parent: Option<String>,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub is_dir: bool,
    pub location: Location,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    Create {
        id: String,
        entry: Entry,
    },
    Move {
        id: String,
        location: Location,
    },
    Delete {
        id: String,
        /// All entries actually observed inside the deleted item. Missing
        /// hashes identify directories or an unobserved concurrent note.
        observed: BTreeMap<String, Option<String>>,
    },
    Restore {
        locations: BTreeMap<String, Location>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Operation {
    pub clock: u64,
    pub author: String,
    pub parents: BTreeSet<String>,
    pub change: Change,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    version: u8,
    #[serde(default)]
    pub seeds: BTreeMap<String, Entry>,
    #[serde(default)]
    pub operations: BTreeMap<String, Operation>,
    #[serde(default)]
    pub acknowledgements: BTreeMap<String, BTreeSet<String>>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            version: 1,
            seeds: BTreeMap::new(),
            operations: BTreeMap::new(),
            acknowledgements: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedEntry {
    pub id: String,
    pub is_dir: bool,
    pub path: String,
    pub location: Location,
    pub aliases: BTreeSet<String>,
    pub deletions: BTreeMap<String, Option<String>>,
    pub path_conflict: bool,
}

impl ResolvedEntry {
    pub fn deleted(&self) -> bool {
        !self.deletions.is_empty()
    }
}

fn valid_id(id: &str, length: usize) -> bool {
    id.len() == length && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_location(location: &Location, is_dir: bool) -> bool {
    let name = &location.name;
    !name.is_empty()
        && name.chars().count() <= 255
        && !name.starts_with('.')
        && !name
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '/' | '\\' | ':'))
        && location.parent.as_ref().is_none_or(|id| valid_id(id, 64))
        && (is_dir || crate::vault::is_markdown(Path::new(name)))
}

pub fn random_id() -> String {
    let bytes: [u8; 32] = rand::random();
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl Catalog {
    pub fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        if bytes.len() > MAX_BYTES {
            bail!("catalog exceeds transfer limit");
        }
        let catalog: Self = serde_json::from_slice(bytes)?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn encode(&self) -> anyhow::Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self)?;
        if bytes.len() > MAX_BYTES {
            bail!("catalog exceeds transfer limit");
        }
        Ok(bytes)
    }

    fn entries(&self) -> anyhow::Result<BTreeMap<String, Entry>> {
        let mut entries = self.seeds.clone();
        for operation in self.operations.values() {
            if let Change::Create { id, entry } = &operation.change {
                if entries.insert(id.clone(), entry.clone()).is_some() {
                    bail!("entry identity was reused");
                }
            }
        }
        Ok(entries)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.version != 1 {
            bail!("unsupported catalog version");
        }
        let entries = self.entries()?;
        let births: BTreeMap<&String, &String> = self
            .operations
            .iter()
            .filter_map(|(op_id, operation)| {
                if let Change::Create { id, .. } = &operation.change {
                    Some((id, op_id))
                } else {
                    None
                }
            })
            .collect();
        for (id, entry) in &entries {
            if !valid_id(id, 64) || !valid_location(&entry.location, entry.is_dir) {
                bail!("invalid catalog entry");
            }
            validate_parent(&entries, &entry.location)?;
        }
        for (id, operation) in &self.operations {
            if !valid_id(id, 64)
                || operation.clock == 0
                || operation.author.is_empty()
                || operation.author.len() > 256
                || operation.author.chars().any(char::is_control)
            {
                bail!("invalid catalog operation");
            }
            for parent in &operation.parents {
                let previous = self
                    .operations
                    .get(parent)
                    .context("catalog operation parent is missing")?;
                if previous.clock >= operation.clock {
                    bail!("invalid catalog causality");
                }
            }
            match &operation.change {
                Change::Create { id, entry } => {
                    if self.seeds.contains_key(id) || !valid_location(&entry.location, entry.is_dir)
                    {
                        bail!("invalid entry creation");
                    }
                }
                Change::Move { id, location } => {
                    let entry = entries.get(id).context("unknown moved entry")?;
                    if !valid_location(location, entry.is_dir) {
                        bail!("invalid entry destination");
                    }
                    validate_parent(&entries, location)?;
                }
                Change::Delete { id, observed } => {
                    entries.get(id).context("unknown deleted entry")?;
                    if !observed.contains_key(id) {
                        bail!("deleted entry is not observed");
                    }
                    for (entry_id, hash) in observed {
                        entries.get(entry_id).context("unknown deletion member")?;
                        if hash.as_ref().is_some_and(|hash| !valid_id(hash, 64)) {
                            bail!("invalid deleted content hash");
                        }
                    }
                }
                Change::Restore { locations } => {
                    if locations.is_empty() {
                        bail!("empty entry restoration");
                    }
                    for (id, location) in locations {
                        let entry = entries.get(id).context("unknown restored entry")?;
                        if !valid_location(location, entry.is_dir) {
                            bail!("invalid restored location");
                        }
                        validate_parent(&entries, location)?;
                    }
                }
            }
            let mut references = Vec::new();
            match &operation.change {
                Change::Create { entry, .. } => references.extend(entry.location.parent.iter()),
                Change::Move { id, location } => {
                    references.push(id);
                    references.extend(location.parent.iter());
                }
                Change::Delete { id, observed } => {
                    references.push(id);
                    references.extend(observed.keys());
                }
                Change::Restore { locations } => {
                    for (id, location) in locations {
                        references.push(id);
                        references.extend(location.parent.iter());
                    }
                }
            }
            if references.iter().any(|entry_id| {
                births
                    .get(entry_id)
                    .is_some_and(|birth| !self.observes(id, birth))
            }) {
                bail!("operation references an unobserved entry creation");
            }
        }
        for (device, acknowledged) in &self.acknowledgements {
            if device.is_empty()
                || device.len() > 256
                || device.chars().any(char::is_control)
                || acknowledged
                    .iter()
                    .any(|id| !self.operations.contains_key(id))
            {
                bail!("invalid catalog acknowledgement");
            }
        }
        Ok(())
    }

    pub fn heads(&self) -> BTreeSet<String> {
        let referenced: BTreeSet<&String> = self
            .operations
            .values()
            .flat_map(|operation| &operation.parents)
            .collect();
        self.operations
            .keys()
            .filter(|id| !referenced.contains(id))
            .cloned()
            .collect()
    }

    pub fn push(&mut self, author: &str, change: Change) -> anyhow::Result<String> {
        let id = random_id();
        let operation = Operation {
            clock: self
                .operations
                .values()
                .map(|op| op.clock)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .context("catalog clock exhausted")?,
            author: author.into(),
            parents: self.heads(),
            change,
        };
        self.operations.insert(id.clone(), operation);
        if let Err(error) = self.validate() {
            self.operations.remove(&id);
            return Err(error);
        }
        Ok(id)
    }

    pub fn merged(&self, remote: &Self) -> anyhow::Result<Self> {
        remote.validate()?;
        let mut merged = self.clone();
        for (id, entry) in &remote.seeds {
            if merged.seeds.get(id).is_some_and(|local| local != entry) {
                bail!("seed identity was reused");
            }
            merged.seeds.insert(id.clone(), entry.clone());
        }
        for (id, operation) in &remote.operations {
            if merged
                .operations
                .get(id)
                .is_some_and(|local| local != operation)
            {
                bail!("operation identity was reused");
            }
            merged.operations.insert(id.clone(), operation.clone());
        }
        for (device, acknowledged) in &remote.acknowledgements {
            merged
                .acknowledgements
                .entry(device.clone())
                .or_default()
                .extend(acknowledged.iter().cloned());
        }
        merged.validate()?;
        Ok(merged)
    }

    pub fn acknowledge(
        &mut self,
        device: &str,
        applied: impl IntoIterator<Item = String>,
    ) -> anyhow::Result<()> {
        let applied: BTreeSet<_> = applied.into_iter().collect();
        if device.is_empty()
            || device.len() > 256
            || device.chars().any(char::is_control)
            || applied.iter().any(|id| !self.operations.contains_key(id))
        {
            bail!("invalid catalog acknowledgement");
        }
        self.acknowledgements
            .entry(device.into())
            .or_default()
            .extend(applied);
        Ok(())
    }

    fn observes(&self, observer: &str, event: &str) -> bool {
        let mut pending = vec![observer];
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if id == event {
                return true;
            }
            if seen.insert(id) {
                if let Some(operation) = self.operations.get(id) {
                    pending.extend(operation.parents.iter().map(String::as_str));
                }
            }
        }
        false
    }

    /// Initial files use a deterministic identity so matching legacy vaults
    /// agree. Intentional recreation after deletion must use Change::Create.
    pub fn seed_path(&mut self, relative: &str, is_dir: bool) -> anyhow::Result<String> {
        let mut parent = None;
        let mut prefix = String::new();
        let parts: Vec<_> = relative.split('/').collect();
        for (index, name) in parts.iter().enumerate() {
            if index > 0 {
                prefix.push('/');
            }
            prefix.push_str(name);
            let directory = index + 1 < parts.len() || is_dir;
            let id = blake3::hash(
                format!("{}:{prefix}", if directory { "directory" } else { "note" }).as_bytes(),
            )
            .to_hex()
            .to_string();
            let entry = Entry {
                is_dir: directory,
                location: Location {
                    parent,
                    name: (*name).into(),
                },
            };
            if !valid_location(&entry.location, directory) {
                bail!("invalid initial entry path");
            }
            if self.seeds.get(&id).is_some_and(|old| old != &entry) {
                bail!("initial entry identity was reused");
            }
            self.seeds.entry(id.clone()).or_insert(entry);
            parent = Some(id);
        }
        parent.context("empty initial entry path")
    }

    pub fn ensure_path(
        &mut self,
        relative: &str,
        is_dir: bool,
        author: &str,
    ) -> anyhow::Result<String> {
        let mut known: BTreeMap<String, String> = self
            .resolve()?
            .into_iter()
            .filter(|(_, entry)| !entry.deleted())
            .map(|(id, entry)| (entry.path, id))
            .collect();
        let mut prefix = String::new();
        let mut parent = None;
        let parts: Vec<_> = relative.split('/').collect();
        for (index, name) in parts.iter().enumerate() {
            if index > 0 {
                prefix.push('/');
            }
            prefix.push_str(name);
            let directory = index + 1 < parts.len() || is_dir;
            if let Some(id) = known.get(&prefix) {
                if self.entries()?[id].is_dir != directory {
                    bail!("catalog path has a different entry type");
                }
                parent = Some(id.clone());
                continue;
            }
            let id = if self.operations.is_empty() {
                self.seed_path(&prefix, directory)?
            } else {
                let id = random_id();
                self.push(
                    author,
                    Change::Create {
                        id: id.clone(),
                        entry: Entry {
                            is_dir: directory,
                            location: Location {
                                parent: parent.clone(),
                                name: (*name).into(),
                            },
                        },
                    },
                )?;
                id
            };
            known.insert(prefix.clone(), id.clone());
            parent = Some(id);
        }
        parent.context("empty catalog path")
    }

    pub fn seed_generated(&mut self, relative: &str) -> anyhow::Result<String> {
        let (parent_path, name) = relative
            .rsplit_once('/')
            .map(|(parent, name)| (Some(parent), name))
            .unwrap_or((None, relative));
        let resolved = self.resolve()?;
        if let Some(entry) = resolved
            .values()
            .find(|entry| !entry.deleted() && entry.path == relative)
        {
            return Ok(entry.id.clone());
        }
        let parent = parent_path
            .map(|path| {
                resolved
                    .values()
                    .find(|entry| !entry.deleted() && entry.is_dir && entry.path == path)
                    .map(|entry| entry.id.clone())
                    .context("generated note parent is unavailable")
            })
            .transpose()?;
        let id = blake3::hash(
            format!("generated:{}:{name}", parent.as_deref().unwrap_or("")).as_bytes(),
        )
        .to_hex()
        .to_string();
        let entry = Entry {
            is_dir: false,
            location: Location {
                parent,
                name: name.into(),
            },
        };
        if self
            .entries()?
            .get(&id)
            .is_some_and(|previous| previous != &entry)
        {
            bail!("generated identity was reused");
        }
        self.seeds.insert(id.clone(), entry);
        self.validate()?;
        Ok(id)
    }

    /// Discover untouched initial files without turning a known stale alias
    /// into a fresh identity. Explicit create/rename actions can use ensure_path.
    pub fn discover_existing(&mut self, root: &Path, author: &str) -> anyhow::Result<()> {
        let resolved = self.resolve()?;
        let known: BTreeSet<String> = resolved
            .values()
            .flat_map(|entry| entry.aliases.iter().cloned())
            .collect();
        let mut paths = Vec::new();
        for entry in walkdir::WalkDir::new(root)
            .min_depth(1)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                let name = entry.file_name().to_str().unwrap_or("");
                !name.starts_with('.') && name != "node_modules" && name != "target"
            })
        {
            let entry = entry?;
            if entry.file_type().is_symlink()
                || (!entry.file_type().is_dir() && !crate::vault::is_markdown(entry.path()))
            {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            if !known.contains(&relative) {
                paths.push((relative, entry.file_type().is_dir()));
            }
        }
        paths.sort_by_key(|(path, _)| (path.matches('/').count(), path.clone()));
        if self.operations.is_empty() {
            for (path, is_dir) in paths {
                self.seed_path(&path, is_dir)?;
            }
        } else {
            for (path, is_dir) in paths {
                self.ensure_path(&path, is_dir, author)?;
            }
        }
        Ok(())
    }

    pub fn resolve(&self) -> anyhow::Result<BTreeMap<String, ResolvedEntry>> {
        self.validate()?;
        let entries = self.entries()?;
        let mut locations: BTreeMap<_, _> = entries
            .iter()
            .map(|(id, entry)| (id.clone(), entry.location.clone()))
            .collect();
        let mut aliases: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        collect_aliases(&entries, &locations, &mut aliases)?;
        let mut operations: Vec<_> = self.operations.iter().collect();
        operations.sort_by_key(|(id, operation)| (operation.clock, *id));
        for (_, operation) in &operations {
            match &operation.change {
                Change::Move { id, location } => {
                    locations.insert(id.clone(), location.clone());
                }
                Change::Restore {
                    locations: restored,
                } => {
                    locations.extend(restored.clone());
                }
                _ => continue,
            }
            collect_aliases(&entries, &locations, &mut aliases)?;
        }
        let (_, _, effective) = resolve_paths(&entries, &locations, &BTreeSet::new())?;
        let births: BTreeMap<&String, &String> = operations
            .iter()
            .filter_map(|(op_id, operation)| {
                if let Change::Create { id, .. } = &operation.change {
                    Some((id, *op_id))
                } else {
                    None
                }
            })
            .collect();
        let mut resolved = BTreeMap::new();
        for (id, entry) in &entries {
            let mut deletions = BTreeMap::new();
            for (delete_id, operation) in &operations {
                let Change::Delete {
                    id: target,
                    observed,
                } = &operation.change
                else {
                    continue;
                };
                let covers = observed.contains_key(id)
                    || (entries[target].is_dir && is_descendant(id, target, &effective));
                if !covers {
                    continue;
                }
                if !observed.contains_key(id)
                    && births
                        .get(id)
                        .is_some_and(|birth| self.observes(birth, delete_id))
                {
                    continue;
                }
                let restored = operations.iter().any(|(restore_id, op)| {
                    matches!(&op.change, Change::Restore { locations } if locations.contains_key(id))
                        && self.observes(restore_id, delete_id)
                });
                if !restored {
                    deletions.insert((*delete_id).clone(), observed.get(id).cloned().flatten());
                }
            }
            resolved.insert(
                id.clone(),
                ResolvedEntry {
                    id: id.clone(),
                    is_dir: entry.is_dir,
                    path: String::new(),
                    location: locations[id].clone(),
                    aliases: aliases.remove(id).unwrap_or_default(),
                    deletions,
                    path_conflict: false,
                },
            );
        }
        // Deleted identities must not reserve a live name. Creating a new note
        // at the old path intentionally gives it a new identity and that path.
        let deleted: BTreeSet<_> = resolved
            .iter()
            .filter(|(_, entry)| entry.deleted())
            .map(|(id, _)| id.clone())
            .collect();
        let (paths, conflicts, _) = resolve_paths(&entries, &locations, &deleted)?;
        for (id, entry) in &mut resolved {
            entry.path = paths[id].clone();
            entry.aliases.insert(entry.path.clone());
            entry.path_conflict = !entry.deleted() && conflicts.contains(id);
        }
        Ok(resolved)
    }
}

fn validate_parent(entries: &BTreeMap<String, Entry>, location: &Location) -> anyhow::Result<()> {
    if let Some(parent) = &location.parent {
        if !entries.get(parent).is_some_and(|entry| entry.is_dir) {
            bail!("entry parent is not a directory");
        }
    }
    Ok(())
}

fn is_descendant(id: &str, ancestor: &str, locations: &BTreeMap<String, Location>) -> bool {
    let mut current = Some(id);
    let mut seen = BTreeSet::new();
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }
        if !seen.insert(id) {
            return false;
        }
        current = locations
            .get(id)
            .and_then(|location| location.parent.as_deref());
    }
    false
}

pub(crate) fn conflict_name(name: &str, id: &str, is_dir: bool, counter: usize) -> String {
    let path = Path::new(name);
    let stem = if is_dir {
        name
    } else {
        path.file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or(name)
    };
    let suffix = format!(" (path conflict {}-{counter})", &id[..16]);
    let extension = if is_dir {
        String::new()
    } else {
        format!(
            ".{}",
            path.extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("md")
        )
    };
    let budget = 255usize.saturating_sub(suffix.len() + extension.len());
    let mut prefix = String::new();
    for ch in stem.chars() {
        if prefix.len() + ch.len_utf8() > budget {
            break;
        }
        prefix.push(ch);
    }
    format!("{prefix}{suffix}{extension}")
}

fn resolve_paths(
    entries: &BTreeMap<String, Entry>,
    locations: &BTreeMap<String, Location>,
    deleted: &BTreeSet<String>,
) -> anyhow::Result<(
    BTreeMap<String, String>,
    BTreeSet<String>,
    BTreeMap<String, Location>,
)> {
    let mut effective = locations.clone();
    let mut conflicts = BTreeSet::new();
    // Concurrent individually-valid moves can form a cycle. Break the same
    // smallest identity on every replica and keep every entry visible.
    for id in entries.keys() {
        let mut chain: Vec<String> = Vec::new();
        let mut current = Some(id.clone());
        while let Some(id) = current {
            if let Some(start) = chain.iter().position(|seen| seen == &id) {
                let smallest = chain[start..].iter().min().unwrap().clone();
                let location = effective
                    .get_mut(&smallest)
                    .context("missing cyclic entry")?;
                location.parent = None;
                location.name =
                    conflict_name(&location.name, &smallest, entries[&smallest].is_dir, 0);
                conflicts.insert(smallest);
                break;
            }
            chain.push(id.clone());
            current = effective
                .get(&id)
                .context("missing entry location")?
                .parent
                .clone();
        }
    }
    let mut groups: BTreeMap<(Option<String>, String), Vec<String>> = BTreeMap::new();
    for (id, location) in &effective {
        if deleted.contains(id) {
            continue;
        }
        groups
            .entry((location.parent.clone(), location.name.to_lowercase()))
            .or_default()
            .push(id.clone());
    }
    let mut reserved: BTreeSet<(Option<String>, String)> = groups.keys().cloned().collect();
    for ((parent, _), ids) in groups {
        for id in ids.into_iter().skip(1) {
            let location = effective.get_mut(&id).unwrap();
            let mut counter = 0;
            loop {
                let name = conflict_name(&location.name, &id, entries[&id].is_dir, counter);
                if reserved.insert((parent.clone(), name.to_lowercase())) {
                    location.name = name;
                    break;
                }
                counter += 1;
            }
            conflicts.insert(id);
        }
    }
    let mut paths: BTreeMap<String, String> = BTreeMap::new();
    for id in entries.keys() {
        let mut pending = Vec::new();
        let mut current = Some(id.clone());
        while let Some(id) = current {
            if paths.contains_key(&id) {
                break;
            }
            pending.push(id.clone());
            current = effective[&id].parent.clone();
        }
        for id in pending.into_iter().rev() {
            let location = &effective[&id];
            let path = match &location.parent {
                Some(parent) => format!("{}/{}", paths[parent], location.name),
                None => location.name.clone(),
            };
            if path.len() > 4096 {
                bail!("resolved catalog path is too long");
            }
            paths.insert(id, path);
        }
    }
    Ok((paths, conflicts, effective))
}

fn collect_aliases(
    entries: &BTreeMap<String, Entry>,
    locations: &BTreeMap<String, Location>,
    aliases: &mut BTreeMap<String, BTreeSet<String>>,
) -> anyhow::Result<()> {
    for (id, path) in resolve_paths(entries, locations, &BTreeSet::new())?.0 {
        aliases.entry(id).or_default().insert(path);
    }
    Ok(())
}

pub fn file_path(root: &Path) -> PathBuf {
    root.join(RELATIVE_PATH)
}

pub fn load(root: &Path) -> anyhow::Result<Catalog> {
    match crate::storage::read_validated(&file_path(root), |bytes| Catalog::decode(bytes).is_ok())?
    {
        Some(bytes) => Catalog::decode(&bytes),
        None => Ok(Catalog::default()),
    }
}

pub fn transact<T>(
    root: &Path,
    update: impl FnOnce(&mut Catalog) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let _guard = LOCK.get_or_init(Mutex::default).lock();
    let mut catalog = load(root)?;
    let result = update(&mut catalog)?;
    crate::storage::write_validated(&file_path(root), &catalog.encode()?, |bytes| {
        Catalog::decode(bytes).is_ok()
    })?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hash(text: &str) -> String {
        blake3::hash(text.as_bytes()).to_hex().to_string()
    }
    fn baseline() -> (Catalog, String) {
        let mut catalog = Catalog::default();
        let id = catalog.seed_path("folder/note.md", false).unwrap();
        (catalog, id)
    }
    fn delete(catalog: &mut Catalog, id: &str, author: &str) -> String {
        catalog
            .push(
                author,
                Change::Delete {
                    id: id.into(),
                    observed: BTreeMap::from([(id.into(), Some(hash("baseline")))]),
                },
            )
            .unwrap()
    }

    #[test]
    fn three_offline_replicas_keep_a_deletion_through_concurrent_renames_and_acknowledge_it() {
        let (base, id) = baseline();
        let mut a = base.clone();
        let mut b = base.clone();
        let mut c = base.clone();
        let deletion = delete(&mut a, &id, "A");
        b.push(
            "B",
            Change::Move {
                id: id.clone(),
                location: Location {
                    parent: None,
                    name: "moved.md".into(),
                },
            },
        )
        .unwrap();
        c.push(
            "C",
            Change::Move {
                id: id.clone(),
                location: Location {
                    parent: None,
                    name: "other.md".into(),
                },
            },
        )
        .unwrap();
        let merged = a.merged(&b).unwrap().merged(&c).unwrap();
        assert_eq!(merged, c.merged(&a).unwrap().merged(&b).unwrap());
        let state = merged.resolve().unwrap();
        assert!(state[&id].deleted());
        assert_eq!(state[&id].deletions[&deletion], Some(hash("baseline")));
        assert!(state[&id].aliases.contains("folder/note.md"));
        let mut acknowledged = merged.clone();
        acknowledged
            .acknowledge("device-A", [deletion.clone()])
            .unwrap();
        let acknowledged = acknowledged.merged(&base).unwrap();
        assert!(acknowledged.acknowledgements["device-A"].contains(&deletion));
        assert_eq!(
            acknowledged.resolve().unwrap()[&id].deletions,
            state[&id].deletions
        );
    }

    #[test]
    fn only_an_observed_restore_revives_a_note_and_recreation_has_a_new_identity() {
        let (base, id) = baseline();
        let mut deleted = base.clone();
        let mut concurrent = base;
        let location = deleted.seeds[&id].location.clone();
        delete(&mut deleted, &id, "A");
        concurrent
            .push(
                "B",
                Change::Restore {
                    locations: BTreeMap::from([(id.clone(), location.clone())]),
                },
            )
            .unwrap();
        let mut merged = deleted.merged(&concurrent).unwrap();
        assert!(merged.resolve().unwrap()[&id].deleted());
        merged
            .push(
                "B",
                Change::Restore {
                    locations: BTreeMap::from([(id.clone(), location.clone())]),
                },
            )
            .unwrap();
        assert!(!merged.resolve().unwrap()[&id].deleted());
        delete(&mut merged, &id, "A");
        let fresh = random_id();
        merged
            .push(
                "B",
                Change::Create {
                    id: fresh.clone(),
                    entry: Entry {
                        is_dir: false,
                        location,
                    },
                },
            )
            .unwrap();
        let state = merged.resolve().unwrap();
        assert!(state[&id].deleted());
        assert!(!state[&fresh].deleted());
        assert_eq!(state[&fresh].path, "folder/note.md");
        assert_ne!(fresh, id);
    }

    #[test]
    fn directory_identity_moves_unknown_offline_children_and_deletes_them_without_a_false_content_hash(
    ) {
        let (base, id) = baseline();
        let folder = base.seeds[&id].location.parent.clone().unwrap();
        let mut a = base.clone();
        let mut b = base;
        a.push(
            "A",
            Change::Move {
                id: folder.clone(),
                location: Location {
                    parent: None,
                    name: "renamed".into(),
                },
            },
        )
        .unwrap();
        let other = b.seed_path("folder/offline.md", false).unwrap();
        let merged = a.merged(&b).unwrap();
        assert_eq!(merged.resolve().unwrap()[&other].path, "renamed/offline.md");
        a.push(
            "A",
            Change::Delete {
                id: folder.clone(),
                observed: BTreeMap::from([(folder, None), (id, Some(hash("baseline")))]),
            },
        )
        .unwrap();
        let merged = a.merged(&b).unwrap();
        let state = merged.resolve().unwrap();
        assert!(state[&other].deleted());
        assert!(state[&other].deletions.values().all(Option::is_none));
    }

    #[test]
    fn path_collisions_and_concurrent_move_cycles_preserve_all_entries_deterministically() {
        let mut base = Catalog::default();
        let a = base.seed_path("A", true).unwrap();
        let b = base.seed_path("B", true).unwrap();
        let note_a = base.seed_path("A/a.md", false).unwrap();
        let note_b = base.seed_path("B/b.md", false).unwrap();
        let mut left = base.clone();
        let mut right = base;
        left.push(
            "left",
            Change::Move {
                id: a.clone(),
                location: Location {
                    parent: Some(b.clone()),
                    name: "A".into(),
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
                        parent: Some(a.clone()),
                        name: "B".into(),
                    },
                },
            )
            .unwrap();
        let combined = left.merged(&right).unwrap();
        let state = combined.resolve().unwrap();
        assert_eq!(state, right.merged(&left).unwrap().resolve().unwrap());
        assert!(state.values().any(|entry| entry.path_conflict));
        assert!(state[&note_a].path.ends_with("/a.md"));
        assert!(state[&note_b].path.ends_with("/b.md"));
        let mut collision = Catalog::default();
        let id_a = collision.seed_path("a.md", false).unwrap();
        let id_b = collision.seed_path("b.md", false).unwrap();
        collision
            .push(
                "left",
                Change::Move {
                    id: id_b.clone(),
                    location: Location {
                        parent: None,
                        name: "a.md".into(),
                    },
                },
            )
            .unwrap();
        let state = collision.resolve().unwrap();
        assert_ne!(state[&id_a].path, state[&id_b].path);
        assert_eq!(
            state.values().filter(|entry| entry.path_conflict).count(),
            1
        );
    }

    #[test]
    fn catalog_and_acknowledgements_survive_restart_and_invalid_updates_keep_the_original() {
        let root = tempfile::tempdir().unwrap();
        let id = transact(root.path(), |catalog| catalog.seed_path("note.md", false)).unwrap();
        let deletion = transact(root.path(), |catalog| Ok(delete(catalog, &id, "A"))).unwrap();
        transact(root.path(), |catalog| {
            catalog.acknowledge("device-B", [deletion.clone()])
        })
        .unwrap();
        let before = std::fs::read(file_path(root.path())).unwrap();
        let loaded = load(root.path()).unwrap();
        assert!(loaded.resolve().unwrap()[&id].deleted());
        assert!(loaded.acknowledgements["device-B"].contains(&deletion));
        assert!(transact(root.path(), |catalog| catalog.push(
            "A",
            Change::Move {
                id: id.clone(),
                location: Location {
                    parent: None,
                    name: "../outside.md".into()
                }
            }
        ))
        .is_err());
        assert_eq!(std::fs::read(file_path(root.path())).unwrap(), before);
        let mut invalid = loaded.clone();
        invalid
            .operations
            .get_mut(&deletion)
            .unwrap()
            .parents
            .insert(deletion.clone());
        assert!(Catalog::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
        assert!(loaded.merged(&invalid).is_err());
    }
}
