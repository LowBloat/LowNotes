//! Reference bindings travel in the note's Yrs state. Delimiter anchors survive
//! edits and concurrent destination replacements; projection uses catalog IDs.
use crate::{
    catalog::{Catalog, ResolvedEntry},
    crdt::CrdtManager,
    references::{self, ReferenceKind},
    vault,
};
use anyhow::{bail, Context};
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::Path,
};
use yrs::{
    updates::decoder::Decode, Any, Assoc, Doc, GetString, IndexedSequence, Map, Out, StickyIndex,
    Text, Transact, Update,
};

const BINDINGS: &str = "lownotes.reference-bindings.v1";
const OPERATIONS: &str = "lownotes.reference-operations.v1";
const URI: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'%')
    .add(b'#')
    .add(b'?')
    .add(b'\\')
    .add(b'"')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'<')
    .add(b'>')
    .add(b'[')
    .add(b']')
    .add(b'|');
const WIKI: &AsciiSet = &CONTROLS
    .add(b'%')
    .add(b'#')
    .add(b'?')
    .add(b'[')
    .add(b']')
    .add(b'|');

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Binding {
    version: u8,
    source: String,
    target: String,
    /// Non-Markdown assets follow their containing directory's identity.
    asset: Option<String>,
    start: StickyIndex,
    end: StickyIndex,
    kind: ReferenceKind,
    root: bool,
    extension: bool,
    suffix: String,
    original: String,
}
#[derive(Serialize, Deserialize)]
struct RewriteOperation {
    binding: String,
    destination: String,
}

fn operations(doc: &Doc) -> anyhow::Result<BTreeMap<yrs::ClientID, RewriteOperation>> {
    let map = doc.get_or_insert_map(OPERATIONS);
    let txn = doc.transact();
    let mut operations = BTreeMap::new();
    for (key, value) in map.iter(&txn) {
        let client: u64 = key.parse().context("invalid reference operation client")?;
        let Out::Any(Any::String(raw)) = value else {
            bail!("invalid reference operation value");
        };
        if client >= 1 << 53 || raw.len() > 16 * 1024 {
            bail!("invalid reference operation");
        }
        let operation: RewriteOperation = serde_json::from_str(&raw)?;
        if !valid_id(&operation.binding)
            || operation.destination.len() > 8192
            || operation.destination.is_empty()
        {
            bail!("invalid reference operation");
        }
        operations.insert(yrs::ClientID::new(client), operation);
    }
    Ok(operations)
}

fn document(state: &[u8], client: Option<u64>) -> anyhow::Result<Doc> {
    // Yjs IDs use UTF-16 clocks. StickyIndex positions must use that same
    // encoding, then be translated to parser byte offsets at the boundary.
    let mut options = yrs::Options::default();
    options.offset_kind = yrs::OffsetKind::Utf16;
    if let Some(client) = client {
        options.client_id = yrs::ClientID::new(client);
    }
    let doc = Doc::with_options(options);
    doc.transact_mut().apply_update(Update::decode_v1(state)?)?;
    Ok(doc)
}
fn client_id(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(blake3::hash(bytes).as_bytes()[..8].try_into().unwrap()) & ((1u64 << 53) - 1)
}
fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn bindings(doc: &Doc) -> anyhow::Result<BTreeMap<String, Binding>> {
    let map = doc.get_or_insert_map(BINDINGS);
    let txn = doc.transact();
    let mut found = BTreeMap::new();
    for (key, value) in map.iter(&txn) {
        let Out::Any(Any::String(raw)) = value else {
            bail!("invalid reference binding value");
        };
        if raw.len() > 64 * 1024 {
            bail!("reference binding is too large");
        }
        let binding: Binding = serde_json::from_str(&raw)?;
        if !valid_id(key)
            || binding.version != 1
            || !valid_id(&binding.source)
            || !valid_id(&binding.target)
            || binding.original.len() > 8192
            || binding
                .asset
                .as_ref()
                .is_some_and(|path| normalize_path(path).as_deref() != Some(path))
        {
            bail!("invalid reference binding");
        }
        found.insert(key.to_string(), binding);
    }
    Ok(found)
}
fn range(doc: &Doc, binding: &Binding) -> Option<Range<usize>> {
    let text = doc.get_or_insert_text("content");
    let txn = doc.transact();
    let start = binding.start.get_offset(&txn)?;
    let end = binding.end.get_offset(&txn)?;
    if start.branch != end.branch || start.branch.as_ref() != text.as_ref() {
        return None;
    }
    let content = text.get_string(&txn);
    let start = byte_index(&content, start.index)?;
    let end = byte_index(&content, end.index)?;
    (start < end).then_some(start..end)
}

fn utf16_index(content: &str, byte: usize) -> Option<u32> {
    content.get(..byte)?.encode_utf16().count().try_into().ok()
}
fn byte_index(content: &str, units: u32) -> Option<usize> {
    let mut offset = 0u32;
    for (byte, ch) in content.char_indices() {
        if offset == units {
            return Some(byte);
        }
        offset += ch.len_utf16() as u32;
        if offset > units {
            return None;
        }
    }
    (offset == units).then_some(content.len())
}
fn root_id() -> String {
    blake3::hash(b"lownotes.reference.vault-root.v1")
        .to_hex()
        .to_string()
}

fn normalize_path(path: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part if part.contains(':')
                || part.contains('\\')
                || part.chars().any(char::is_control) =>
            {
                return None
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}
fn split_destination(destination: &str) -> Option<(String, String)> {
    let destination = destination.trim();
    let position = destination.find(['#', '?']).unwrap_or(destination.len());
    let path = percent_decode_str(&destination[..position])
        .decode_utf8()
        .ok()?
        .into_owned();
    if path.is_empty() || path.contains(':') || path.starts_with("//") {
        return None;
    }
    Some((path, destination[position..].into()))
}
fn relative(source: &str, target: &str) -> String {
    let source: Vec<_> = source
        .rsplit_once('/')
        .map_or("", |(parent, _)| parent)
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let target: Vec<_> = target.split('/').collect();
    let shared = source
        .iter()
        .zip(&target)
        .take_while(|(left, right)| left == right)
        .count();
    std::iter::repeat_n("..", source.len() - shared)
        .chain(target[shared..].iter().copied())
        .collect::<Vec<_>>()
        .join("/")
}
fn render(binding: &Binding, source: &str, target: &str) -> String {
    let target = binding.asset.as_ref().map_or_else(
        || target.to_string(),
        |asset| {
            if target.is_empty() {
                asset.clone()
            } else {
                format!("{target}/{asset}")
            }
        },
    );
    let path = match binding.kind {
        ReferenceKind::Wiki => {
            if binding.extension {
                target
            } else {
                target
                    .strip_suffix(".markdown")
                    .or_else(|| target.strip_suffix(".md"))
                    .unwrap_or(&target)
                    .into()
            }
        }
        ReferenceKind::Markdown if binding.root => format!("/{target}"),
        ReferenceKind::Markdown => relative(source, &target),
    };
    format!(
        "{}{}",
        utf8_percent_encode(
            &path,
            if binding.kind == ReferenceKind::Wiki {
                WIKI
            } else {
                URI
            }
        ),
        binding.suffix
    )
}

fn resolve(
    root: &Path,
    entries: &BTreeMap<String, ResolvedEntry>,
    paths: &BTreeMap<String, String>,
    source: &str,
    destination: &str,
    kind: &ReferenceKind,
) -> Option<(String, Option<String>)> {
    let (clean, _) = split_destination(destination)?;
    let parent = source.rsplit_once('/').map_or("", |(parent, _)| parent);
    let relative = format!("{parent}/{clean}");
    let candidates = if clean.starts_with('/') {
        vec![clean.trim_start_matches('/').into()]
    } else if kind == &ReferenceKind::Wiki
        && clean.contains('/')
        && !clean.starts_with("./")
        && !clean.starts_with("../")
    {
        vec![clean.clone(), relative]
    } else {
        vec![relative, clean.clone()]
    };
    for candidate in candidates {
        let Some(candidate) = normalize_path(&candidate) else {
            continue;
        };
        let mut candidates = vec![candidate.clone()];
        if kind == &ReferenceKind::Wiki && !vault::is_markdown(Path::new(&candidate)) {
            candidates.push(format!("{candidate}.md"));
        }
        for candidate in candidates {
            let matching: Vec<_> = entries
                .iter()
                .filter(|(id, entry)| {
                    !entry.deleted()
                        && !entry.is_dir
                        && paths
                            .get(*id)
                            .unwrap_or(&entry.path)
                            .eq_ignore_ascii_case(&candidate)
                })
                .collect();
            if matching.len() == 1 {
                return Some((matching[0].0.clone(), None));
            }
        }
        // Relative binary assets are bound only when the file exists. An
        // arbitrary unresolved URL must never be rewritten into another file.
        if kind == &ReferenceKind::Markdown && root.join(&candidate).is_file() {
            let containing = entries
                .iter()
                .filter(|(_, entry)| !entry.deleted() && entry.is_dir)
                .filter_map(|(id, entry)| {
                    let directory = paths.get(id).unwrap_or(&entry.path);
                    candidate
                        .strip_prefix(&format!("{directory}/"))
                        .map(|suffix| (directory.len(), id.clone(), suffix.to_string()))
                })
                .max_by_key(|(length, _, _)| *length);
            if let Some((_, id, asset)) = containing {
                return Some((id, Some(asset)));
            }
            return Some((root_id(), Some(candidate)));
        }
    }
    if kind == &ReferenceKind::Wiki && !clean.contains('/') {
        let title = clean
            .strip_suffix(".markdown")
            .or_else(|| clean.strip_suffix(".md"))
            .unwrap_or(&clean);
        let matching: Vec<_> = entries
            .iter()
            .filter(|(id, entry)| {
                if entry.deleted() || entry.is_dir {
                    return false;
                }
                let path = paths.get(*id).unwrap_or(&entry.path);
                vault::read_note(root, path).is_ok_and(|content| {
                    content
                        .lines()
                        .take(10)
                        .find_map(|line| {
                            line.trim()
                                .strip_prefix("# ")
                                .map(str::trim)
                                .filter(|title| !title.is_empty())
                        })
                        .is_some_and(|heading| heading.eq_ignore_ascii_case(title))
                })
            })
            .collect();
        if matching.len() == 1 {
            return Some((matching[0].0.clone(), None));
        }
    }
    None
}

fn bind_state(
    root: &Path,
    state: &[u8],
    source: &str,
    path: &str,
    entries: &BTreeMap<String, ResolvedEntry>,
    paths: &BTreeMap<String, String>,
) -> anyhow::Result<Vec<u8>> {
    let mut doc = document(state, None)?;
    let content = doc
        .get_or_insert_text("content")
        .get_string(&doc.transact());
    let existing = bindings(&doc)?;
    let existing_operations = operations(&doc)?;
    let occupied: BTreeSet<_> = existing
        .iter()
        .filter(|(_, binding)| binding.source == source)
        .filter_map(|(key, binding)| {
            let span = range(&doc, binding)?;
            let allowed = spellings(binding, key, entries, &existing_operations);
            recognized(content.get(span.clone())?, &allowed).then_some((span.start, span.end))
        })
        .collect();
    let mut seen = BTreeSet::new();
    for reference in references::scan(&content) {
        let span = reference.destination_range;
        if span.len() > 8192
            || occupied.contains(&(span.start, span.end))
            || !seen.insert((span.start, span.end))
        {
            continue;
        }
        let Some((target, asset)) = resolve(
            root,
            entries,
            paths,
            path,
            &reference.destination,
            &reference.kind,
        ) else {
            continue;
        };
        let Some((clean, suffix)) = split_destination(&reference.destination) else {
            continue;
        };
        let text = doc.get_or_insert_text("content");
        let txn = doc.transact();
        let Some(start) = text.sticky_index(
            &txn,
            utf16_index(&content, span.start).context("invalid reference boundary")?,
            Assoc::Before,
        ) else {
            continue;
        };
        let end_index = utf16_index(&content, span.end).context("invalid reference boundary")?;
        let end = text
            .sticky_index(&txn, end_index, Assoc::After)
            .unwrap_or_else(|| StickyIndex::from_type(&txn, &text, Assoc::After));
        drop(txn);
        let binding = Binding {
            version: 1,
            source: source.into(),
            target,
            asset,
            start,
            end,
            kind: reference.kind,
            root: clean.starts_with('/'),
            extension: vault::is_markdown(Path::new(&clean)),
            suffix,
            original: content[span].into(),
        };
        let raw = serde_json::to_string(&binding)?;
        let key = blake3::hash(raw.as_bytes()).to_hex().to_string();
        let next = document(
            &CrdtManager::encode_state(&doc),
            Some(client_id(format!("bind:{key}").as_bytes())),
        )?;
        next.get_or_insert_map(BINDINGS)
            .insert(&mut next.transact_mut(), key, raw);
        doc = next;
    }
    Ok(CrdtManager::encode_state(&doc))
}

/// True when the range consists only of legitimate spellings of the same
/// identity. Concurrent replacements can temporarily concatenate spellings.
fn recognized(raw: &str, allowed: &BTreeSet<String>) -> bool {
    if raw.len() > 8192 {
        return false;
    }
    let mut reachable = BTreeSet::from([0usize]);
    for index in 0..raw.len() {
        if !reachable.contains(&index) || !raw.is_char_boundary(index) {
            continue;
        }
        for spelling in allowed {
            if !spelling.is_empty() && raw[index..].starts_with(spelling) {
                reachable.insert(index + spelling.len());
            }
        }
    }
    reachable.contains(&raw.len())
}

fn spellings(
    binding: &Binding,
    key: &str,
    entries: &BTreeMap<String, ResolvedEntry>,
    operations: &BTreeMap<yrs::ClientID, RewriteOperation>,
) -> BTreeSet<String> {
    let mut allowed = BTreeSet::from([binding.original.clone()]);
    allowed.extend(
        operations
            .values()
            .filter(|operation| operation.binding == key)
            .map(|operation| operation.destination.clone()),
    );
    if let Some(source) = entries.get(&binding.source) {
        let paths = entries.get(&binding.target).map_or_else(
            || BTreeSet::from([String::new()]),
            |target| {
                let mut paths = target.aliases.clone();
                paths.insert(target.path.clone());
                paths
            },
        );
        for source_path in source.aliases.iter().chain(std::iter::once(&source.path)) {
            for target_path in &paths {
                allowed.insert(render(binding, source_path, target_path));
            }
        }
    }
    allowed
}

fn normalize_state(
    state: &[u8],
    source: &str,
    entries: &BTreeMap<String, ResolvedEntry>,
) -> anyhow::Result<Vec<u8>> {
    let mut doc = document(state, None)?;
    let source_entry = entries
        .get(source)
        .context("reference source identity is unavailable")?;
    if source_entry.deleted() {
        return Ok(state.into());
    }
    for (key, binding) in bindings(&doc)? {
        if binding.source != source {
            continue;
        }
        let target = entries
            .get(&binding.target)
            .filter(|target| !target.deleted());
        let root_asset = binding.target == root_id() && binding.asset.is_some();
        if !root_asset && target.is_none_or(|target| target.is_dir != binding.asset.is_some()) {
            continue;
        }
        let target_path = target.map_or("", |target| target.path.as_str());
        let Some(span) = range(&doc, &binding) else {
            continue;
        };
        let text = doc.get_or_insert_text("content");
        let content = text.get_string(&doc.transact());
        let Some(raw) = content.get(span.clone()) else {
            continue;
        };
        // Removed syntax, code examples and deliberate changes to another
        // destination are not an invitation to recreate the original link.
        if !references::scan(&content)
            .iter()
            .any(|reference| reference.destination_range == span && reference.kind == binding.kind)
        {
            continue;
        }
        let replacement = render(&binding, &source_entry.path, target_path);
        if raw == replacement {
            continue;
        }
        let mut allowed = spellings(&binding, &key, entries, &operations(&doc)?);
        allowed.insert(replacement.clone());
        if !recognized(raw, &allowed) {
            continue;
        }
        // Derive the operation ID from the actual characters, rather than the
        // whole document. Unrelated offline edits do not duplicate the URI.
        let txn = doc.transact();
        let from = utf16_index(&content, span.start).context("invalid reference boundary")?;
        let length = raw.encode_utf16().count() as u32;
        let anchors: Vec<_> = raw
            .char_indices()
            .filter_map(|(offset, _)| {
                text.sticky_index(
                    &txn,
                    from + raw[..offset].encode_utf16().count() as u32,
                    Assoc::After,
                )
            })
            .collect();
        let seed = serde_json::to_vec(&("reference-rewrite-v1", &key, &anchors, &replacement))?;
        drop(txn);
        let next = document(&CrdtManager::encode_state(&doc), Some(client_id(&seed)))?;
        let text = next.get_or_insert_text("content");
        let mut txn = next.transact_mut();
        text.remove_range(&mut txn, from, length);
        text.insert(&mut txn, from, &replacement);
        drop(txn);
        next.get_or_insert_map(OPERATIONS).insert(
            &mut next.transact_mut(),
            client_id(&seed).to_string(),
            serde_json::to_string(&RewriteOperation {
                binding: key,
                destination: replacement,
            })?,
        );
        doc = next;
    }
    Ok(CrdtManager::encode_state(&doc))
}

pub(crate) fn bind_all(
    root: &Path,
    manager: &CrdtManager,
    catalog: &Catalog,
    paths: &BTreeMap<String, String>,
) -> anyhow::Result<()> {
    let entries = catalog.resolve()?;
    for (id, entry) in &entries {
        if entry.deleted() || entry.is_dir {
            continue;
        }
        let path = paths.get(id).unwrap_or(&entry.path);
        if !root.join(path).is_file() {
            continue;
        }
        if references::scan(&vault::read_note(root, path)?).is_empty() {
            continue;
        }
        manager.transform_state(root, path, |state| {
            bind_state(root, state, id, path, &entries, paths)
        })?;
    }
    Ok(())
}
pub(crate) fn normalize_snapshot(root: &Path, path: &str, state: &[u8]) -> anyhow::Result<Vec<u8>> {
    if !state
        .windows(BINDINGS.len())
        .any(|bytes| bytes == BINDINGS.as_bytes())
    {
        return Ok(state.into());
    }
    if !crate::catalog::file_path(root).exists() {
        return Ok(state.into());
    }
    let entries = crate::catalog::load(root)?.resolve()?;
    let Some(source) = entries
        .values()
        .find(|entry| !entry.deleted() && !entry.is_dir && entry.path == path)
    else {
        return Ok(state.into());
    };
    normalize_state(state, &source.id, &entries)
}
/// Reference maintenance changes URI characters, but is not a competing user
/// edit. It must not turn a sole offline user edit into an artificial conflict.
pub(crate) fn structural_clients(doc: &Doc) -> anyhow::Result<BTreeSet<yrs::ClientID>> {
    let mut clients: BTreeSet<_> = bindings(doc)?
        .keys()
        .map(|key| yrs::ClientID::new(client_id(format!("bind:{key}").as_bytes())))
        .collect();
    clients.extend(operations(doc)?.into_keys());
    Ok(clients)
}
pub(crate) fn normalize_all(
    root: &Path,
    manager: &CrdtManager,
    catalog: &Catalog,
) -> anyhow::Result<()> {
    let entries = catalog.resolve()?;
    for (id, entry) in &entries {
        if entry.deleted() || entry.is_dir || !root.join(&entry.path).is_file() {
            continue;
        }
        let Some(bytes) =
            CrdtManager::read_state_file(root, &CrdtManager::state_relative_path(&entry.path))?
        else {
            continue;
        };
        if !bytes
            .windows(BINDINGS.len())
            .any(|bytes| bytes == BINDINGS.as_bytes())
        {
            continue;
        }
        manager.transform_state(root, &entry.path, |state| {
            normalize_state(state, id, &entries)
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{catalog_sync, structural};
    use std::fs;

    fn fixture() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("folder")).unwrap();
        fs::write(root.path().join("folder/target.md"), "# Destino\n").unwrap();
        fs::write(root.path().join("folder/source.md"), "É 🙂 [**rótulo**](target.md#seção \"Título\")\r\n[[folder/target#etapa|alias]]\n[referência][id]\n\n[id]: <target.md#ref> 'título'\n\n`[exemplo](target.md)`\n~~~~\n[[folder/target]]\n~~~~\n![foto](../asset.png)\n").unwrap();
        fs::write(root.path().join("asset.png"), [0, 255, 32]).unwrap();
        root
    }
    fn identity(root: &Path, path: &str) -> String {
        crate::catalog::load(root)
            .unwrap()
            .resolve()
            .unwrap()
            .values()
            .find(|entry| !entry.deleted() && entry.path == path)
            .unwrap()
            .id
            .clone()
    }
    fn read(root: &Path, path: &str) -> String {
        vault::read_note(root, path).unwrap()
    }

    #[test]
    fn note_and_folder_moves_rewrite_destinations_preserving_labels_titles_anchors_and_code() {
        let root = fixture();
        let manager = CrdtManager::new();
        catalog_sync::prepare(root.path(), &manager, "local").unwrap();
        let original = manager
            .get_or_create_doc(root.path(), "folder/source.md")
            .unwrap();
        structural::rename(
            root.path(),
            "folder/target.md",
            "folder/a b.md",
            &manager,
            "local",
        )
        .unwrap();
        let first = read(root.path(), "folder/source.md");
        assert!(
            first.contains("[**rótulo**](a%20b.md#seção \"Título\")\r\n"),
            "{first}"
        );
        assert!(first.contains("[[folder/a b#etapa|alias]]"), "{first}");
        assert!(first.contains("[id]: <a%20b.md#ref> 'título'"), "{first}");
        assert!(first.contains("`[exemplo](target.md)`\n~~~~\n[[folder/target]]\n~~~~"));
        structural::rename(root.path(), "folder", "moved", &manager, "local").unwrap();
        assert!(read(root.path(), "moved/source.md").contains("[[moved/a b#etapa|alias]]"));
        structural::rename(
            root.path(),
            "moved/source.md",
            "source.md",
            &manager,
            "local",
        )
        .unwrap();
        let result = read(root.path(), "source.md");
        assert!(
            result.contains("[**rótulo**](moved/a%20b.md#seção \"Título\")"),
            "{result}"
        );
        assert!(
            result.contains("[id]: <moved/a%20b.md#ref> 'título'"),
            "{result}"
        );
        let after = manager.get_or_create_doc(root.path(), "source.md").unwrap();
        let old = document(&original, None).unwrap();
        let new = document(&after, None).unwrap();
        use yrs::ReadTxn;
        for (client, clock) in old.transact().state_vector().iter() {
            assert!(new.transact().state_vector().get(client) >= *clock);
        }
        assert!(crate::links::graph_links(root.path())
            .unwrap()
            .iter()
            .any(|edge| edge.source == "source.md" && edge.target == "moved/a b.md"));
    }

    #[test]
    fn moved_notes_update_outgoing_asset_links_even_when_the_asset_does_not_move() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("notes")).unwrap();
        fs::create_dir_all(root.path().join("assets")).unwrap();
        fs::write(root.path().join("assets/image.png"), [1, 2, 3]).unwrap();
        fs::write(
            root.path().join("notes/note.md"),
            "![foto](../assets/image.png \"legenda\")\n",
        )
        .unwrap();
        let manager = CrdtManager::new();
        structural::rename(root.path(), "notes/note.md", "note.md", &manager, "local").unwrap();
        assert_eq!(
            read(root.path(), "note.md"),
            "![foto](assets/image.png \"legenda\")\n"
        );
        structural::rename(root.path(), "assets", "pictures", &manager, "local").unwrap();
        assert_eq!(
            read(root.path(), "note.md"),
            "![foto](pictures/image.png \"legenda\")\n"
        );
    }

    #[test]
    fn delimiter_anchors_follow_concurrent_surrounding_edits_and_identical_rewrites_deduplicate() {
        let root = fixture();
        let manager = CrdtManager::new();
        let catalog = catalog_sync::prepare(root.path(), &manager, "local").unwrap();
        let source = identity(root.path(), "folder/source.md");
        let target = identity(root.path(), "folder/target.md");
        let state = manager
            .get_or_create_doc(root.path(), "folder/source.md")
            .unwrap();
        let left = document(&state, Some(123)).unwrap();
        left.get_or_insert_text("content")
            .insert(&mut left.transact_mut(), 0, "Antes 🙂\n");
        let right = document(&state, Some(456)).unwrap();
        right
            .get_or_insert_text("content")
            .push(&mut right.transact_mut(), "\nDepois\n");
        let mut renamed = catalog.clone();
        renamed
            .push(
                "move",
                crate::catalog::Change::Move {
                    id: target,
                    location: crate::catalog::Location {
                        parent: Some(identity(root.path(), "folder")),
                        name: "new.md".into(),
                    },
                },
            )
            .unwrap();
        let entries = renamed.resolve().unwrap();
        let left = normalize_state(&CrdtManager::encode_state(&left), &source, &entries).unwrap();
        let right = normalize_state(&CrdtManager::encode_state(&right), &source, &entries).unwrap();
        let merged = document(&left, None).unwrap();
        merged
            .transact_mut()
            .apply_update(Update::decode_v1(&right).unwrap())
            .unwrap();
        let result = merged
            .get_or_insert_text("content")
            .get_string(&merged.transact());
        assert!(result.starts_with("Antes 🙂\n"));
        assert!(result.ends_with("\nDepois\n"));
        assert!(result.contains("(new.md#seção \"Título\")"), "{result}");
        assert!(!result.contains("new.md#seçãonew.md#seção"), "{result}");
        assert_eq!(
            normalize_state(&CrdtManager::encode_state(&merged), &source, &entries).unwrap(),
            CrdtManager::encode_state(&merged)
        );
    }

    #[test]
    fn different_offline_destinations_are_projected_after_merge_without_duplicate_urls() {
        let root = fixture();
        let manager = CrdtManager::new();
        let catalog = catalog_sync::prepare(root.path(), &manager, "local").unwrap();
        let source = identity(root.path(), "folder/source.md");
        let target = identity(root.path(), "folder/target.md");
        let state = manager
            .get_or_create_doc(root.path(), "folder/source.md")
            .unwrap();
        let mut a = catalog.clone();
        let mut b = catalog.clone();
        for (catalog, author, name) in [(&mut a, "a", "one.md"), (&mut b, "b", "two.md")] {
            catalog
                .push(
                    author,
                    crate::catalog::Change::Move {
                        id: target.clone(),
                        location: crate::catalog::Location {
                            parent: Some(identity(root.path(), "folder")),
                            name: name.into(),
                        },
                    },
                )
                .unwrap();
        }
        let left = normalize_state(&state, &source, &a.resolve().unwrap()).unwrap();
        let right = normalize_state(&state, &source, &b.resolve().unwrap()).unwrap();
        let merged = document(&left, None).unwrap();
        merged
            .transact_mut()
            .apply_update(Update::decode_v1(&right).unwrap())
            .unwrap();
        let union = a.merged(&b).unwrap().resolve().unwrap();
        let merged_content = merged
            .get_or_insert_text("content")
            .get_string(&merged.transact());
        for binding in bindings(&merged).unwrap().values() {
            let span = range(&merged, binding).unwrap();
            let Some(target_entry) = union.get(&binding.target) else {
                continue;
            };
            let mut allowed = BTreeSet::from([binding.original.clone()]);
            for source_path in &union[&source].aliases {
                for target_path in &target_entry.aliases {
                    allowed.insert(render(binding, source_path, target_path));
                }
            }
            assert!(
                references::scan(&merged_content)
                    .iter()
                    .any(|reference| reference.destination_range == span),
                "missing parser range {span:?}: {:?}",
                references::scan(&merged_content)
            );
            assert!(
                recognized(&merged_content[span.clone()], &allowed),
                "unrecognized {:?}: {allowed:?}",
                &merged_content[span]
            );
        }
        let result = document(
            &normalize_state(&CrdtManager::encode_state(&merged), &source, &union).unwrap(),
            None,
        )
        .unwrap();
        let content = result
            .get_or_insert_text("content")
            .get_string(&result.transact());
        let target = union[&target].path.strip_prefix("folder/").unwrap();
        assert!(
            content.contains(&format!("({target}#seção \"Título\")")),
            "{content}"
        );
        assert!(
            content.contains(&format!(
                "[[folder/{}#etapa|alias]]",
                target.trim_end_matches(".md")
            )),
            "{content}"
        );
        assert_eq!(
            references::scan(&content)
                .iter()
                .filter(|reference| !reference.definition && !reference.image)
                .count(),
            3
        );
    }

    #[test]
    fn deliberate_destination_changes_and_removed_links_are_not_overwritten() {
        let root = fixture();
        let manager = CrdtManager::new();
        catalog_sync::prepare(root.path(), &manager, "local").unwrap();
        let content = read(root.path(), "folder/source.md")
            .replace("target.md#seção", "https://example.org/#seção")
            .replace("[[folder/target#etapa|alias]]", "texto simples");
        manager
            .replace_note_text(root.path(), "folder/source.md", &content)
            .unwrap();
        structural::rename(
            root.path(),
            "folder/target.md",
            "folder/new.md",
            &manager,
            "local",
        )
        .unwrap();
        let result = read(root.path(), "folder/source.md");
        assert!(result.contains("https://example.org/#seção"), "{result}");
        assert!(result.contains("texto simples"));
        assert!(!result.contains("[[folder/new#etapa"));
        assert!(result.contains("[id]: <new.md#ref>"), "{result}");
    }

    #[test]
    fn a_reference_deliberately_retargeted_to_another_note_tracks_that_new_identity() {
        let root = fixture();
        let manager = CrdtManager::new();
        fs::write(root.path().join("folder/other.md"), "# Outra\n").unwrap();
        catalog_sync::prepare(root.path(), &manager, "local").unwrap();
        let content =
            read(root.path(), "folder/source.md").replace("target.md#seção", "other.md#seção");
        manager
            .replace_note_text(root.path(), "folder/source.md", &content)
            .unwrap();
        structural::rename(
            root.path(),
            "folder/other.md",
            "folder/final.md",
            &manager,
            "local",
        )
        .unwrap();
        assert!(read(root.path(), "folder/source.md").contains("(final.md#seção \"Título\")"));
        structural::rename(
            root.path(),
            "folder/target.md",
            "folder/old-target.md",
            &manager,
            "local",
        )
        .unwrap();
        assert!(read(root.path(), "folder/source.md").contains("(final.md#seção \"Título\")"));
    }
    #[test]
    fn a_definition_at_end_of_file_is_rewritten_and_a_later_append_remains_intact() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        fs::write(root.path().join("note.md"), "[id]: old.md").unwrap();
        fs::write(root.path().join("old.md"), "# Destino").unwrap();
        structural::rename(root.path(), "old.md", "new.md", &manager, "local").unwrap();
        assert_eq!(read(root.path(), "note.md"), "[id]: new.md");
        manager
            .replace_note_text(root.path(), "note.md", "[id]: new.md\n\nTexto novo 🙂\n")
            .unwrap();
        structural::rename(root.path(), "new.md", "final.md", &manager, "local").unwrap();
        assert_eq!(
            read(root.path(), "note.md"),
            "[id]: final.md\n\nTexto novo 🙂\n"
        );
    }
}
