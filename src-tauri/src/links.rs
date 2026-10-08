use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

use crate::{link_operations::LinkChanges, vault};

// Serialize read/modify/write, including graph reconciliation and receipt of
// remote operations. File replacement alone cannot protect a read-modify-write.
static CHANGES_LOCK: OnceLock<parking_lot::Mutex<()>> = OnceLock::new();

fn changes_lock() -> parking_lot::MutexGuard<'static, ()> {
    CHANGES_LOCK.get_or_init(parking_lot::Mutex::default).lock()
}

pub const LINKS_VERSION: u8 = 1;
pub const LINKS_REL_PATH: &str = ".lownotes/links.json";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
#[allow(non_camel_case_types)]
pub enum LinkOrigin {
    wikilink,
    manual,
    agent,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
#[allow(non_camel_case_types)]
pub enum LinkAction {
    add,
    remove,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LinkEdge {
    pub source: String,
    pub target: String,
    pub origin: LinkOrigin,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LinkOperation {
    pub source: String,
    pub target: String,
    pub action: LinkAction,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LinkStore {
    pub version: u8,
    #[serde(default)]
    pub links: Vec<LinkEdge>,
}

impl Default for LinkStore {
    fn default() -> Self {
        Self {
            version: LINKS_VERSION,
            links: Vec::new(),
        }
    }
}

impl LinkStore {
    pub fn new() -> Self {
        Self::default()
    }
}

pub fn links_path(vault: &Path) -> PathBuf {
    vault.join(".lownotes").join("links.json")
}

/// Read the current projection. Callers that mutate must use the checked path.
pub fn load_links(vault: &Path) -> LinkStore {
    let _guard = changes_lock();
    read_changes(vault).map(|(store, _)| store).unwrap_or_default()
}

fn valid_store(bytes: &[u8]) -> bool {
    serde_json::from_slice::<LinkStore>(bytes).is_ok_and(|store| store.version == LINKS_VERSION)
}

pub fn save_links(vault: &Path, store: &LinkStore) -> anyhow::Result<()> {
    let _guard = changes_lock();
    let (_, mut history) = read_changes(vault)?;
    let catalog = crate::catalog::load(vault)?;
    let entries = catalog.resolve()?;
    for (_, edge) in history.projected_additions(&entries) {
        if !store.links.contains(&edge) { history.remove_projected(&entries, &edge.source, &edge.target); }
    }
    for edge in &store.links { history.add_bound(edge.clone(), &catalog)?; }
    history.bind_current(&catalog)?;
    persist_changes(vault, store, &history)?;
    Ok(())
}

fn operations_path(vault: &Path) -> PathBuf {
    vault.join(crate::link_operations::RELATIVE_PATH)
}

fn project(vault: &Path, store: &LinkStore, history: &LinkChanges) -> anyhow::Result<LinkStore> {
    let entries = crate::catalog::load(vault)?.resolve()?;
    let mut links: Vec<_> = history.projected_additions(&entries).into_iter().map(|(_, edge)| edge).collect();
    for edge in store.links.iter().filter(|edge| edge.origin == LinkOrigin::wikilink) {
        if !links.iter().any(|known| known.source == edge.source && known.target == edge.target) {
            links.push(edge.clone());
        }
    }
    links.sort(); links.dedup();
    Ok(LinkStore { version: LINKS_VERSION, links })
}

fn read_changes(vault: &Path) -> anyhow::Result<(LinkStore, LinkChanges)> {
    let operations = crate::storage::read_validated(&operations_path(vault), |bytes| LinkChanges::decode(bytes).is_ok())?;
    let (cached, damaged_cache) = match crate::storage::read_validated(&links_path(vault), valid_store) {
        Ok(Some(bytes)) => (serde_json::from_slice::<LinkStore>(&bytes)?, false),
        Ok(None) => (LinkStore::new(), false),
        Err(error) if operations.is_none() => return Err(error),
        Err(_) => (LinkStore::new(), true),
    };
    let history = match operations {
        Some(bytes) => LinkChanges::decode(&bytes)?,
        None => { let mut history = LinkChanges::default(); history.import_legacy(&cached.links); history }
    };
    // Missing operations are a one-time legacy migration. Do not import the
    // projection once a journal exists, as its old contents can be stale.
    let projected = project(vault, &cached, &history)?;
    if damaged_cache {
        crate::storage::write_validated(&links_path(vault), &serde_json::to_vec_pretty(&projected)?, valid_store)?;
        crate::storage::report_recovery(&links_path(vault), true);
    }
    // Ensure callers cannot bypass validation with malformed legacy paths.
    history.validate()?;
    Ok((projected, history))
}

fn persist_changes(vault: &Path, store: &LinkStore, history: &LinkChanges) -> anyhow::Result<()> {
    // Commit the authority first. If projection replacement is interrupted,
    // the next read reconstructs the same links from the operation journal.
    crate::storage::write_validated(&operations_path(vault), &history.encode()?, |bytes| LinkChanges::decode(bytes).is_ok())?;
    let projected = project(vault, store, history)?;
    crate::storage::write_validated(&links_path(vault), &serde_json::to_vec_pretty(&projected)?, valid_store)?;
    Ok(())
}

pub fn prepare_sync(vault: &Path) -> anyhow::Result<()> {
    let _guard = changes_lock();
    if !links_path(vault).exists() && !operations_path(vault).exists() { return Ok(()); }
    let (store, history) = read_changes(vault)?;
    persist_changes(vault, &store, &history)
}

pub fn bind_identities(vault: &Path, catalog: &crate::catalog::Catalog) -> anyhow::Result<()> {
    let _guard = changes_lock();
    if !links_path(vault).exists() && !operations_path(vault).exists() { return Ok(()); }
    let (store, mut history) = read_changes(vault)?;
    history.bind_current(catalog)?;
    persist_changes(vault, &store, &history)
}

pub fn is_sync_metadata(path: &str) -> bool {
    path == LINKS_REL_PATH || path == crate::link_operations::RELATIVE_PATH
}

pub fn merge_sync(vault: &Path, path: &str, bytes: &[u8]) -> anyhow::Result<bool> {
    if !is_sync_metadata(path) { bail!("invalid link metadata path"); }
    // Reject a malformed packet before touching any local projection or journal.
    let remote_history = if path == crate::link_operations::RELATIVE_PATH {
        Some(LinkChanges::decode(bytes)?)
    } else {
        if bytes.len() > crate::link_operations::MAX_BYTES || !valid_store(bytes) { bail!("invalid link projection"); }
        None
    };
    let _guard = changes_lock();
    let (store, history) = read_changes(vault)?;
    let mut merged = history.clone();
    if let Some(remote) = remote_history { merged = history.merged(&remote)?; }
    else { merged.import_legacy(&serde_json::from_slice::<LinkStore>(bytes)?.links); }
    let changed = merged != history;
    persist_changes(vault, &store, &merged)?;
    Ok(changed)
}

/// Scan `[[...]]` tokens, skipping fenced code blocks and inline code spans.
/// Returns `(token, span_start, span_end)` with byte offsets into `content`.
fn wikilink_spans(content: &str) -> Vec<(String, usize, usize)> {
    crate::references::scan(content).into_iter()
        .filter(|reference| reference.kind == crate::references::ReferenceKind::Wiki && !reference.image)
        .map(|reference| {
            let span = reference.source_range;
            (content[span.start + 2..span.end - 2].trim().to_string(), span.start, span.end)
        }).collect()
}

/// Extract wikilink tokens from note content, deduplicated, preserving order.
pub fn extract_wikilinks(content: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (token, _, _) in wikilink_spans(content) {
        if !out.iter().any(|t| *t == token) {
            out.push(token);
        }
    }
    out
}

/// Local Markdown links in ordinary prose (code spans and fences are ignored).
fn markdown_link_spans(content: &str) -> Vec<(String, usize, usize, String)> {
    crate::references::scan(content).into_iter()
        .filter(|reference| reference.kind == crate::references::ReferenceKind::Markdown
            && !reference.image && !reference.definition
            && crate::vault::is_markdown(Path::new(reference.destination.split(['#', '?']).next().unwrap_or(""))))
        .map(|reference| (reference.destination, reference.source_range.start, reference.source_range.end, reference.label))
        .collect()
}

fn note_tokens(content: &str) -> Vec<String> {
    crate::references::scan(content).into_iter().filter(|reference| !reference.image && !reference.definition)
        .filter_map(|reference| {
            if reference.kind == crate::references::ReferenceKind::Wiki { return Some(reference.destination); }
            let destination = reference.destination;
            let path = destination.split(['#', '?']).next().unwrap_or("");
            if path.contains(':') || !crate::vault::is_markdown(Path::new(path)) { return None; }
            Some(if path.starts_with('/') || path.starts_with("../") || path.starts_with("./") { destination }
                 else { format!("./{destination}") })
        }).collect()
}

fn normalized_path(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

fn resolve_from_items(items: &[vault::VaultItem], source: &str, token: &str) -> Option<String> {
    let clean = token
        .split('|')
        .next()?
        .split(['#', '?'])
        .next()?
        .trim()
        .trim_matches(['<', '>'])
        .replace('\\', "/");
    let clean = percent_encoding::percent_decode_str(&clean).decode_utf8().ok()?.into_owned();
    if clean.is_empty() || clean.contains("://") {
        return None;
    }
    let parent = source.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
    let mut candidates = Vec::new();
    if clean.starts_with('/') {
        candidates.push(clean.trim_start_matches('/').to_string());
    } else if clean.starts_with("./") || clean.starts_with("../") {
        candidates.push(format!("{parent}/{clean}"));
    } else if clean.contains('/') {
        candidates.push(clean.clone());
        candidates.push(format!("{parent}/{clean}"));
    } else {
        candidates.push(format!("{parent}/{clean}"));
        candidates.push(clean.clone());
    }
    for candidate in candidates {
        let Some(path) = normalized_path(&candidate) else {
            continue;
        };
        let lower = path.to_lowercase();
        let with_ext = if lower.ends_with(".md") || lower.ends_with(".markdown") {
            lower.clone()
        } else {
            format!("{lower}.md")
        };
        if let Some(item) = items.iter().find(|i| {
            !i.is_dir && (i.path.to_lowercase() == lower || i.path.to_lowercase() == with_ext)
        }) {
            return Some(item.path.clone());
        }
    }
    if clean.contains('/') {
        return None;
    }
    let title = clean.to_lowercase();
    let title = title
        .strip_suffix(".markdown")
        .or_else(|| title.strip_suffix(".md"))
        .unwrap_or(&title);
    let mut matches = items
        .iter()
        .filter(|i| !i.is_dir && i.title.to_lowercase() == title);
    let first = matches.next()?;
    if matches.next().is_none() {
        Some(first.path.clone())
    } else {
        None
    }
}

/// Resolve a note token from the vault root. Content links use the source-aware resolver above.
pub fn resolve_link_target(vault: &Path, token: &str) -> Option<String> {
    let items = vault::list_vault_items(vault).ok()?;
    resolve_from_items(&items, "", token)
}

/// Read the graph from current files, so links to notes saved later are visible immediately.
pub fn graph_links(vault: &Path) -> anyhow::Result<Vec<LinkEdge>> {
    let items = vault::list_vault_items(vault)?;
    let exists = |path: &str| items.iter().any(|i| !i.is_dir && i.path == path);
    let store = { let _guard = changes_lock(); read_changes(vault)?.0 };
    let mut edges: Vec<LinkEdge> = store
        .links
        .into_iter()
        .filter(|e| {
            e.origin != LinkOrigin::wikilink
                && exists(&e.source)
                && exists(&e.target)
                && e.source != e.target
        })
        .collect();
    // One visible relation per pair; the operation history retains every
    // origin. Prefer a manual relation when a manual/assistant pair coincides.
    edges.sort_by_key(|edge| (edge.source.clone(), edge.target.clone(), edge.origin != LinkOrigin::manual));
    edges.dedup_by(|a, b| a.source == b.source && a.target == b.target);
    for item in items.iter().filter(|i| !i.is_dir) {
        let Ok(content) = vault::read_note(vault, &item.path) else {
            continue;
        };
        for token in note_tokens(&content) {
            let Some(target) = resolve_from_items(&items, &item.path, &token) else {
                continue;
            };
            if target != item.path
                && !edges
                    .iter()
                    .any(|e| e.source == item.path && e.target == target)
            {
                edges.push(LinkEdge {
                    source: item.path.clone(),
                    target,
                    origin: LinkOrigin::wikilink,
                });
            }
        }
    }
    Ok(edges)
}

/// Rebuild wikilink-origin edges for `source` from its current content.
/// Manual/agent edges are never touched. Saves only when the store changed.
pub fn reconcile_wikilinks(vault: &Path, source: &str, content: &str) -> anyhow::Result<()> {
    // Read note items before holding the metadata mutex: recovery of a note
    // can itself reconcile links.
    let items = vault::list_vault_items(vault)?;
    let _guard = changes_lock();
    let (mut store, history) = read_changes(vault)?;
    let before = store.links.clone();

    store
        .links
        .retain(|e| !(e.origin == LinkOrigin::wikilink && e.source == source));

    for token in note_tokens(content) {
        let Some(target) = resolve_from_items(&items, source, &token) else {
            continue;
        };
        if target == source {
            continue;
        }
        if store
            .links
            .iter()
            .any(|e| e.source == source && e.target == target)
        {
            continue;
        }
        store.links.push(LinkEdge {
            source: source.to_string(),
            target,
            origin: LinkOrigin::wikilink,
        });
    }

    if store.links != before {
        persist_changes(vault, &store, &history)?;
    }
    Ok(())
}

fn validate_note(vault: &Path, relative: &str) -> anyhow::Result<()> {
    let full = vault::safe_join(vault, relative)?;
    if !full.is_file() || !vault::is_markdown(&full) {
        bail!("errors.noteNotFound");
    }
    Ok(())
}

/// Keep visible prose when unlinking a note from the graph.
fn strip_links_from_items(items: &[vault::VaultItem], source: &str, content: &str, target: &str) -> Option<String> {
    let mut replacements: Vec<(usize, usize, String)> = wikilink_spans(content)
        .into_iter()
        .filter(|(token, _, _)| {
            resolve_from_items(items, source, token).as_deref() == Some(target)
        })
        .map(|(token, start, end)| {
            let label = if let Some((_, alias)) = token.split_once('|') {
                alias.to_string()
            } else {
                token
                    .split(['#', '?'])
                    .next()
                    .unwrap_or("")
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .trim_end_matches(".md")
                    .to_string()
            };
            (start, end, label)
        })
        .collect();
    replacements.extend(
        markdown_link_spans(content)
            .into_iter()
            .filter(|(token, _, _, _)| {
                let token = if token.starts_with('/') || token.starts_with("../") || token.starts_with("./") {
                    token.clone()
                } else { format!("./{token}") };
                resolve_from_items(items, source, &token).as_deref() == Some(target)
            })
            .map(|(_, start, end, label)| (start, end, label)),
    );
    if replacements.is_empty() {
        return None;
    }
    replacements.sort_by_key(|(start, _, _)| *start);
    let mut updated = content.to_string();
    for (start, end, label) in replacements.into_iter().rev() {
        updated.replace_range(start..end, &label);
    }
    Some(updated)
}

/// Apply add/remove link operations. Removing an edge also unwraps matching
/// wikilinks and local Markdown links in the source note, preserving their labels.
pub fn apply_operations(
    vault: &Path,
    ops: &[LinkOperation],
    origin: LinkOrigin,
) -> anyhow::Result<()> {
    apply_operations_with_manager(vault, ops, origin, &crate::crdt::CrdtManager::new())
}

const PENDING_LINKS: &str = ".lownotes/pending-links.json";
const MAX_PENDING_BYTES: usize = 24 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct PendingLinks {
    version: u8,
    history: LinkChanges,
    removals: Vec<(String, String)>,
}

fn parse_pending(bytes: &[u8]) -> anyhow::Result<PendingLinks> {
    if bytes.len() > MAX_PENDING_BYTES { bail!("link transaction exceeds local limit"); }
    let pending: PendingLinks = serde_json::from_slice(bytes)?;
    let valid_id = |id: &str| id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit());
    if pending.version != 1 || pending.removals.len() > 10_000
        || pending.removals.iter().any(|(source, target)| !valid_id(source) || !valid_id(target)) {
        bail!("invalid pending link transaction");
    }
    pending.history.validate()?;
    pending.history.encode()?;
    Ok(pending)
}

/// Called by the structural coordinator after Markdown/CRDT and moves recover.
pub(crate) fn recover_pending(vault: &Path, manager: &crate::crdt::CrdtManager) -> anyhow::Result<()> {
    let file = vault.join(PENDING_LINKS);
    for candidate in [&file, &crate::storage::backup_path(&file)] {
        if fs::metadata(candidate).is_ok_and(|metadata| metadata.len() > MAX_PENDING_BYTES as u64) {
            crate::storage::report_recovery(&file, false);
            bail!("link transaction exceeds local limit");
        }
    }
    let Some(bytes) = crate::storage::read_validated(&file, |bytes| parse_pending(bytes).is_ok())? else { return Ok(()); };
    finish_pending(vault, &parse_pending(&bytes)?, manager, &|_| Ok(()))?;
    crate::storage::report_recovery(&file, true);
    Ok(())
}

fn finish_pending(vault: &Path, pending: &PendingLinks, manager: &crate::crdt::CrdtManager, hook: &impl Fn(u8) -> anyhow::Result<()>) -> anyhow::Result<()> {
    let entries = crate::catalog::load(vault)?.resolve()?;
    {
        let _guard = changes_lock();
        let (mut store, history) = read_changes(vault)?;
        let merged = history.merged(&pending.history)?;
        for (source, target) in &pending.removals {
            if let (Some(source), Some(target)) = (entries.get(source), entries.get(target)) {
                store.links.retain(|edge| edge.origin != LinkOrigin::wikilink || edge.source != source.path || edge.target != target.path);
            }
        }
        persist_changes(vault, &store, &merged)?;
    }
    hook(1)?;
    // Re-read the current text rather than replaying an old whole-note snapshot.
    // This retains external edits and follows both endpoints' stable identities.
    let items = vault::list_vault_items(vault)?;
    for (source, target) in &pending.removals {
        let source = entries.get(source).context("pending link source identity is unavailable")?;
        let target = entries.get(target).context("pending link target identity is unavailable")?;
        if source.deleted() || target.deleted() { continue; }
        let content = vault::read_note(vault, &source.path)?;
        if let Some(updated) = strip_links_from_items(&items, &source.path, &content, &target.path) {
            manager.replace_note_text_with_hook(vault, &source.path, &updated, |at| hook(at + 2))?;
        }
        reconcile_wikilinks(vault, &source.path, &vault::read_note(vault, &source.path)?)?;
        manager.notify_projection(vault, &source.path, &manager.get_or_create_doc(vault, &source.path)?);
        hook(5)?;
    }
    hook(6)?;
    let file = vault.join(PENDING_LINKS);
    crate::storage::remove_file(&crate::storage::backup_path(&file))?;
    hook(7)?;
    crate::storage::remove_file(&file)?;
    Ok(())
}

pub(crate) fn apply_operations_with_manager(vault: &Path, ops: &[LinkOperation], origin: LinkOrigin, manager: &crate::crdt::CrdtManager) -> anyhow::Result<()> {
    apply_with_hook(vault, ops, origin, manager, |_| Ok(()))
}

fn apply_with_hook(vault: &Path, ops: &[LinkOperation], origin: LinkOrigin, manager: &crate::crdt::CrdtManager, hook: impl Fn(u8) -> anyhow::Result<()>) -> anyhow::Result<()> {
    crate::structural::exclusive(vault, manager, || apply_inner(vault, ops, origin, manager, &hook))
}

fn apply_inner(vault: &Path, ops: &[LinkOperation], origin: LinkOrigin, manager: &crate::crdt::CrdtManager, hook: &impl Fn(u8) -> anyhow::Result<()>) -> anyhow::Result<()> {
    if ops.len() > 10_000 { bail!("link transaction exceeds local limit"); }
    for op in ops {
        validate_note(vault, &op.source)?;
        validate_note(vault, &op.target)?;
    }

    // Add-only batches remain usable with older metadata-only peers, which do
    // not exchange a structural catalog. Text removals require stable identities.
    let catalog = if ops.iter().any(|op| op.action == LinkAction::remove) {
        crate::catalog_sync::prepare(vault, manager, "links")?
    } else { crate::catalog::load(vault)? };
    let entries = catalog.resolve()?;
    let mut wikilink_removals: Vec<(String, String)> = Vec::new();
    let guard = changes_lock();
    let (mut store, mut history) = read_changes(vault)?;
    for op in ops {
        match op.action {
            LinkAction::add => {
                let exists = store
                    .links
                    .iter()
                    .any(|e| e.source == op.source && e.target == op.target);
                if !exists {
                    let edge = LinkEdge {
                        source: op.source.clone(),
                        target: op.target.clone(),
                        origin: origin.clone(),
                    };
                    history.add_bound(edge.clone(), &catalog)?;
                    store.links.push(edge);
                }
            }
            LinkAction::remove => {
                history.remove_projected(&entries, &op.source, &op.target);
                store
                    .links
                    .retain(|e| !(e.source == op.source && e.target == op.target));
                let source = entries.values().find(|entry| !entry.deleted() && entry.path == op.source).context("link source identity is unavailable")?;
                let target = entries.values().find(|entry| !entry.deleted() && entry.path == op.target).context("link target identity is unavailable")?;
                wikilink_removals.push((source.id.clone(), target.id.clone()));
            }
        }
    }

    history.bind_current(&catalog)?;
    if wikilink_removals.is_empty() {
        return persist_changes(vault, &store, &history);
    }
    wikilink_removals.sort(); wikilink_removals.dedup();
    let pending = PendingLinks { version: 1, history, removals: wikilink_removals };
    let bytes = serde_json::to_vec(&pending)?;
    crate::storage::write_validated(&vault.join(PENDING_LINKS), &bytes, |bytes| parse_pending(bytes).is_ok())?;
    drop(guard);
    hook(0)?;
    finish_pending(vault, &pending, manager, hook)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crdt::CrdtManager;
    use yrs::{updates::decoder::Decode, Doc, GetString, ReadTxn, Text, Transact, Update};

    fn decode_doc(state: &[u8]) -> Doc {
        let doc = Doc::new();
        doc.transact_mut().apply_update(Update::decode_v1(state).unwrap()).unwrap();
        doc
    }

    fn remove_b() -> Vec<LinkOperation> {
        vec![LinkOperation { source: "a.md".into(), target: "b.md".into(), action: LinkAction::remove }]
    }

    fn unlink_fixture(root: &Path, manager: &CrdtManager) -> Vec<u8> {
        fs::write(root.join("a.md"), "# A\nSee [[B|label]] and [target](b.md).\n").unwrap();
        fs::write(root.join("b.md"), "# B\n").unwrap();
        crate::catalog_sync::prepare(root, manager, "local").unwrap();
        manager.get_or_create_doc(root, "a.md").unwrap()
    }

    #[test]
    fn unlink_updates_live_crdt_preserves_history_and_unseen_peer_edits() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        let before = unlink_fixture(root.path(), &manager);
        let remote = decode_doc(&before);
        remote.get_or_insert_text("content").push(&mut remote.transact_mut(), "Remote addition 🙂\n");
        let observed = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let notifications = observed.clone();
        manager.set_projection_observer(std::sync::Arc::new(move |_, path, state| {
            notifications.lock().push((path.to_string(), state.to_vec()));
        }));
        apply_operations_with_manager(root.path(), &remove_b(), LinkOrigin::manual, &manager).unwrap();
        let after = manager.get_or_create_doc(root.path(), "a.md").unwrap();
        let doc = decode_doc(&after);
        assert_eq!(doc.get_or_insert_text("content").get_string(&doc.transact()), "# A\nSee label and target.\n");
        for (client, clock) in decode_doc(&before).transact().state_vector().iter() {
            assert!(doc.transact().state_vector().get(client) >= *clock);
        }
        assert_eq!(observed.lock().last().unwrap().0, "a.md");
        assert_eq!(observed.lock().last().unwrap().1, after);
        assert!(!graph_links(root.path()).unwrap().iter().any(|edge| edge.target == "b.md"));
        let restarted = CrdtManager::new();
        restarted.apply_update(root.path(), "a.md", &CrdtManager::encode_state(&remote)).unwrap();
        let text = vault::read_note(root.path(), "a.md").unwrap();
        assert!(text.contains("See label and target."));
        assert!(text.contains("Remote addition 🙂"));
        assert!(!text.contains("[[B"));
        assert!(!crate::note_history::list(root.path(), &crate::catalog::load(root.path()).unwrap().resolve().unwrap().values().find(|entry| entry.path == "a.md").unwrap().id).unwrap().is_empty());
        assert!(!root.path().join(PENDING_LINKS).exists());
    }

    #[test]
    fn unlink_recovers_after_actual_process_exit_at_each_commit_boundary() {
        for phase in 0..=7 {
            let root = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "links::tests::unlink_crash_worker", "--ignored", "--nocapture"])
                .env("LOWNOTES_UNLINK_ROOT", root.path())
                .env("LOWNOTES_UNLINK_PHASE", phase.to_string()).output().unwrap();
            assert_eq!(output.status.code(), Some(86), "phase={phase}: {}", String::from_utf8_lossy(&output.stderr));
            // Normal vault opening must recover without a special repair command.
            vault::list_vault_items(root.path()).unwrap();
            let manager = CrdtManager::new();
            let after = manager.get_or_create_doc(root.path(), "a.md").unwrap();
            let doc = decode_doc(&after);
            assert_eq!(vault::read_note(root.path(), "a.md").unwrap(), "# A\nSee label and target.\n");
            assert_eq!(doc.get_or_insert_text("content").get_string(&doc.transact()), vault::read_note(root.path(), "a.md").unwrap());
            for (client, clock) in decode_doc(&fs::read(root.path().join(".base.bin")).unwrap()).transact().state_vector().iter() {
                assert!(doc.transact().state_vector().get(client) >= *clock);
            }
            assert!(!graph_links(root.path()).unwrap().iter().any(|edge| edge.target == "b.md"));
            assert!(!root.path().join(PENDING_LINKS).exists());
            assert!(!crate::storage::backup_path(&root.path().join(PENDING_LINKS)).exists());
            vault::list_vault_items(root.path()).unwrap();
            assert_eq!(manager.get_or_create_doc(root.path(), "a.md").unwrap(), after);
        }
    }

    #[test]
    #[ignore = "isolated worker invoked by unlink recovery test"]
    fn unlink_crash_worker() {
        let root = PathBuf::from(std::env::var_os("LOWNOTES_UNLINK_ROOT").unwrap());
        let phase: u8 = std::env::var("LOWNOTES_UNLINK_PHASE").unwrap().parse().unwrap();
        let manager = CrdtManager::new();
        fs::write(root.join(".base.bin"), unlink_fixture(&root, &manager)).unwrap();
        apply_with_hook(&root, &remove_b(), LinkOrigin::manual, &manager, |at| {
            if at == phase { std::process::exit(86); } Ok(())
        }).unwrap();
        panic!("unlink crash point was not reached");
    }

    #[test]
    fn interrupted_unlink_keeps_external_text_and_reports_a_blocked_write() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        unlink_fixture(root.path(), &manager);
        let error = apply_with_hook(root.path(), &remove_b(), LinkOrigin::manual, &manager, |at| {
            if at == 1 {
                fs::rename(root.path().join("a.md"), root.path().join("saved-original.md"))?;
                fs::create_dir(root.path().join("a.md"))?;
            }
            Ok(())
        }).unwrap_err();
        assert!(!error.to_string().is_empty());
        assert!(root.path().join(PENDING_LINKS).exists());
        assert!(fs::read_to_string(root.path().join("saved-original.md")).unwrap().contains("[[B"));
        fs::remove_dir(root.path().join("a.md")).unwrap();
        fs::rename(root.path().join("saved-original.md"), root.path().join("a.md")).unwrap();
        fs::write(root.path().join("a.md"), "# A\nSee [[B|label]] and [target](b.md).\nExternal edit\n").unwrap();
        // External tools may preserve old timestamps; recover directly from the
        // current Markdown before importing its CRDT projection.
        filetime::set_file_mtime(root.path().join("a.md"), filetime::FileTime::from_unix_time(1, 0)).unwrap();
        vault::list_vault_items(root.path()).unwrap();
        assert_eq!(vault::read_note(root.path(), "a.md").unwrap(), "# A\nSee label and target.\nExternal edit\n");
        assert!(!root.path().join(PENDING_LINKS).exists());
    }

    #[test]
    fn damaged_unlink_intent_recovers_a_valid_backup_and_refuses_silent_defaults() {
        for has_backup in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let manager = CrdtManager::new();
            unlink_fixture(root.path(), &manager);
            assert!(apply_with_hook(root.path(), &remove_b(), LinkOrigin::manual, &manager, |at| {
                if at == 0 { bail!("interrupted before metadata"); } Ok(())
            }).is_err());
            let pending = root.path().join(PENDING_LINKS);
            if has_backup { fs::copy(&pending, crate::storage::backup_path(&pending)).unwrap(); }
            fs::write(&pending, b"damaged pending removal").unwrap();
            if has_backup {
                vault::list_vault_items(root.path()).unwrap();
                assert_eq!(vault::read_note(root.path(), "a.md").unwrap(), "# A\nSee label and target.\n");
                assert!(!pending.exists());
                assert!(fs::read_dir(root.path().join(".lownotes")).unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().contains("pending-links.json.corrupt-")));
            } else {
                assert!(vault::list_vault_items(root.path()).is_err());
                assert_eq!(fs::read(&pending).unwrap(), b"damaged pending removal");
                assert!(vault::read_note(root.path(), "a.md").unwrap().contains("[[B|label]]"));
                assert!(apply_operations(root.path(), &[], LinkOrigin::manual).is_err());
            }
        }
    }

    #[test]
    fn concurrent_local_map_actions_keep_every_link_and_journal_survives_a_stale_projection() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a.md"), "# A\n").unwrap();
        for index in 0..20 { fs::write(root.path().join(format!("{index}.md")), "target").unwrap(); }
        std::thread::scope(|scope| {
            for index in 0..20 {
                let vault = root.path();
                scope.spawn(move || apply_operations(vault, &[LinkOperation {
                    source: "a.md".into(), target: format!("{index}.md"), action: LinkAction::add,
                }], LinkOrigin::manual).unwrap());
            }
        });
        assert_eq!(graph_links(root.path()).unwrap().len(), 20);
        let expected = fs::read(operations_path(root.path())).unwrap();
        // Simulates process termination after committing operations but before
        // replacing the projection. Reading/manifest preparation repairs it.
        fs::write(links_path(root.path()), br#"{"version":1,"links":[]}"#).unwrap();
        assert_eq!(load_links(root.path()).links.len(), 20);
        prepare_sync(root.path()).unwrap();
        assert_eq!(serde_json::from_slice::<LinkStore>(&fs::read(links_path(root.path())).unwrap()).unwrap().links.len(), 20);
        assert_eq!(fs::read(operations_path(root.path())).unwrap(), expected);
    }

    #[test]
    fn invalid_remote_metadata_cannot_replace_local_operations_or_projection() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a.md"), "# A\n").unwrap();
        fs::write(root.path().join("b.md"), "# B\n").unwrap();
        apply_operations(root.path(), &[LinkOperation { source: "a.md".into(), target: "b.md".into(), action: LinkAction::add }], LinkOrigin::manual).unwrap();
        let projection = fs::read(links_path(root.path())).unwrap();
        let operations = fs::read(operations_path(root.path())).unwrap();
        let mut remote = LinkChanges::decode(&operations).unwrap();
        remote.additions.values_mut().next().unwrap().target = "c.md".into();
        assert!(merge_sync(root.path(), crate::link_operations::RELATIVE_PATH, &remote.encode().unwrap()).is_err());
        assert!(merge_sync(root.path(), crate::link_operations::RELATIVE_PATH, b"truncated").is_err());
        assert!(merge_sync(root.path(), LINKS_REL_PATH, br#"{"version":99,"links":[]}"#).is_err());
        assert_eq!(fs::read(operations_path(root.path())).unwrap(), operations);
        assert_eq!(fs::read(links_path(root.path())).unwrap(), projection);
    }

    #[test]
    fn corrupt_links_recover_valid_relationships_and_cannot_overwrite_unrecoverable_data() {
        let root = tempfile::tempdir().unwrap();
        let mut store = LinkStore::new();
        store.links.push(LinkEdge { source: "a.md".into(), target: "b.md".into(), origin: LinkOrigin::manual });
        save_links(root.path(), &store).unwrap();
        store.links.push(LinkEdge { source: "b.md".into(), target: "c.md".into(), origin: LinkOrigin::agent });
        save_links(root.path(), &store).unwrap();
        fs::write(links_path(root.path()), b"truncated").unwrap();
        // The journal is authoritative even when the recovered projection's
        // backup predates the latest addition.
        assert_eq!(load_links(root.path()), store);
        fs::write(links_path(root.path()), b"broken again").unwrap();
        fs::remove_file(crate::storage::backup_path(&links_path(root.path()))).unwrap();
        assert_eq!(load_links(root.path()), store, "valid operations rebuild a corrupt projection");
        fs::write(operations_path(root.path()), b"broken operations").unwrap();
        fs::remove_file(crate::storage::backup_path(&operations_path(root.path()))).unwrap();
        assert!(save_links(root.path(), &LinkStore::new()).is_err());
        assert_eq!(fs::read(operations_path(root.path())).unwrap(), b"broken operations");
    }

    fn temp_vault(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lownotes-links-test-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_link_store_roundtrip() {
        let vault = temp_vault("roundtrip");
        // Missing file => empty store, version 1.
        let empty = load_links(&vault);
        assert_eq!(empty.version, 1);
        assert!(empty.links.is_empty());

        let store = LinkStore {
            version: LINKS_VERSION,
            links: vec![
                LinkEdge {
                    source: "a.md".to_string(),
                    target: "b.md".to_string(),
                    origin: LinkOrigin::wikilink,
                },
                LinkEdge {
                    source: "a.md".to_string(),
                    target: "c.md".to_string(),
                    origin: LinkOrigin::manual,
                },
                LinkEdge {
                    source: "b.md".to_string(),
                    target: "c.md".to_string(),
                    origin: LinkOrigin::agent,
                },
            ],
        };
        save_links(&vault, &store).unwrap();
        assert!(links_path(&vault).is_file());
        let loaded = load_links(&vault);
        assert_eq!(loaded, store);

        // A corrupt projection is rebuilt from its valid operation history.
        fs::write(links_path(&vault), "{ not json").unwrap();
        let corrupt = load_links(&vault);
        assert_eq!(corrupt.version, 1);
        assert_eq!(corrupt.links.len(), 2);
        assert!(corrupt.links.iter().all(|edge| edge.origin != LinkOrigin::wikilink));

        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn test_extract_wikilinks_skips_code() {
        let content = "\
# Title\n\
Link to [[Alpha]] and [[beta note]] and again [[Alpha]].\n\
Inline `code with [[NotALink]]` stays out.\n\
```md\n\
fenced [[AlsoNotALink]]\n\
```\n\
After fence [[Gamma]].\n";
        let tokens = extract_wikilinks(content);
        assert_eq!(tokens, vec!["Alpha", "beta note", "Gamma"]);
    }

    #[test]
    fn test_reconcile_wikilinks_keeps_manual_and_agent() {
        let vault = temp_vault("reconcile");
        fs::write(vault.join("a.md"), "# A\n\nSee [[B]].\n").unwrap();
        fs::write(vault.join("b.md"), "# B\n").unwrap();

        // Seed a manual edge that must survive reconciliation.
        let seeded = LinkStore {
            version: LINKS_VERSION,
            links: vec![LinkEdge {
                source: "a.md".to_string(),
                target: "b.md".to_string(),
                origin: LinkOrigin::manual,
            }],
        };
        save_links(&vault, &seeded).unwrap();

        reconcile_wikilinks(
            &vault,
            "a.md",
            &fs::read_to_string(vault.join("a.md")).unwrap(),
        )
        .unwrap();
        let store = load_links(&vault);
        // Pair already exists (manual) so no duplicate wikilink edge is added.
        assert_eq!(store.links.len(), 1);
        assert_eq!(store.links[0].origin, LinkOrigin::manual);

        // Remove the manual edge, reconcile adds the wikilink one.
        save_links(&vault, &LinkStore::new()).unwrap();
        reconcile_wikilinks(&vault, "a.md", "See [[B]] and [[Missing Note]].").unwrap();
        let store = load_links(&vault);
        assert_eq!(
            store.links,
            vec![LinkEdge {
                source: "a.md".to_string(),
                target: "b.md".to_string(),
                origin: LinkOrigin::wikilink,
            }]
        );

        // Content without the wikilink removes the edge.
        reconcile_wikilinks(&vault, "a.md", "No links here.").unwrap();
        let store = load_links(&vault);
        assert!(store.links.is_empty());

        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn test_apply_operations_add_and_remove_strips_token() {
        let vault = temp_vault("apply");
        fs::write(vault.join("a.md"), "# A\n\nSee [[B]] for details.\n").unwrap();
        fs::write(vault.join("b.md"), "# B\n").unwrap();
        fs::write(vault.join("c.md"), "# C\n").unwrap();

        // Add manual edge a -> c.
        apply_operations(
            &vault,
            &[LinkOperation {
                source: "a.md".to_string(),
                target: "c.md".to_string(),
                action: LinkAction::add,
            }],
            LinkOrigin::manual,
        )
        .unwrap();
        // Adding the same pair again is ignored (no duplicates).
        apply_operations(
            &vault,
            &[LinkOperation {
                source: "a.md".to_string(),
                target: "c.md".to_string(),
                action: LinkAction::add,
            }],
            LinkOrigin::agent,
        )
        .unwrap();
        let store = load_links(&vault);
        assert_eq!(store.links.len(), 1);
        assert_eq!(store.links[0].origin, LinkOrigin::manual);

        // Reconcile wikilink a -> b, then remove it via operations: the
        // [[B]] token must disappear from a.md and the edge from the store.
        reconcile_wikilinks(
            &vault,
            "a.md",
            &fs::read_to_string(vault.join("a.md")).unwrap(),
        )
        .unwrap();
        assert!(load_links(&vault)
            .links
            .iter()
            .any(|e| e.target == "b.md" && e.origin == LinkOrigin::wikilink));

        apply_operations(
            &vault,
            &[LinkOperation {
                source: "a.md".to_string(),
                target: "b.md".to_string(),
                action: LinkAction::remove,
            }],
            LinkOrigin::manual,
        )
        .unwrap();

        let store = load_links(&vault);
        assert!(!store.links.iter().any(|e| e.target == "b.md"));
        // Manual edge untouched.
        assert!(store
            .links
            .iter()
            .any(|e| e.target == "c.md" && e.origin == LinkOrigin::manual));
        let content = fs::read_to_string(vault.join("a.md")).unwrap();
        assert!(!content.contains("[[B]]"));
        assert!(content.contains("for details."));

        // Validation: unknown note fails.
        let err = apply_operations(
            &vault,
            &[LinkOperation {
                source: "a.md".to_string(),
                target: "nope.md".to_string(),
                action: LinkAction::add,
            }],
            LinkOrigin::manual,
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "errors.noteNotFound");

        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn test_build_manifest_includes_links_file() {
        let vault = temp_vault("manifest");
        fs::write(vault.join("a.md"), "# A\n").unwrap();
        let store = LinkStore {
            version: LINKS_VERSION,
            links: vec![LinkEdge {
                source: "a.md".to_string(),
                target: "a.md".to_string(),
                origin: LinkOrigin::manual,
            }],
        };
        save_links(&vault, &store).unwrap();

        let manifest = vault::build_manifest(&vault).unwrap();
        let meta = manifest
            .get(LINKS_REL_PATH)
            .expect("links.json missing from manifest");
        assert_eq!(meta.path, LINKS_REL_PATH);
        assert!(!meta.hash.is_empty());
        assert!(meta.size > 0);

        // UI listing must NOT expose .lownotes.
        let items = vault::list_vault_items(&vault).unwrap();
        assert!(!items.iter().any(|i| i.path.contains(".lownotes")));

        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn graph_finds_late_targets_and_relative_markdown_links() {
        let vault = temp_vault("late-targets");
        fs::create_dir_all(vault.join("Python/Etapas")).unwrap();
        fs::write(
            vault.join("Python/Plano.md"),
            "# Plano\n\n[[Etapas/Fundamentos|Começar]]\n",
        )
        .unwrap();
        assert!(graph_links(&vault).unwrap().is_empty());
        fs::write(
            vault.join("Python/Etapas/Fundamentos.md"),
            "# Fundamentos\n\n[Voltar](../Plano.md#Objetivos)\n`[ignorar](../Plano.md)`\n",
        )
        .unwrap();
        let edges = graph_links(&vault).unwrap();
        assert_eq!(edges.len(), 2);
        assert!(edges
            .iter()
            .any(|e| e.source == "Python/Plano.md" && e.target == "Python/Etapas/Fundamentos.md"));
        assert!(edges
            .iter()
            .any(|e| e.source == "Python/Etapas/Fundamentos.md" && e.target == "Python/Plano.md"));
        assert_eq!(
            resolve_link_target(&vault, "Python/Plano"),
            Some("Python/Plano.md".into())
        );
        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn unlink_derived_edge_keeps_visible_text() {
        let vault = temp_vault("unlink-derived");
        fs::write(
            vault.join("a.md"),
            "Veja [[B|o plano]] e [detalhes](B.md).\n",
        )
        .unwrap();
        fs::write(vault.join("B.md"), "# B\n").unwrap();
        assert_eq!(graph_links(&vault).unwrap().len(), 1);
        assert!(load_links(&vault).links.is_empty());
        apply_operations(
            &vault,
            &[LinkOperation {
                source: "a.md".into(),
                target: "B.md".into(),
                action: LinkAction::remove,
            }],
            LinkOrigin::manual,
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(vault.join("a.md")).unwrap(),
            "Veja o plano e detalhes.\n"
        );
        assert!(graph_links(&vault).unwrap().is_empty());
        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn manual_links_follow_both_endpoints_and_removal_survives_old_metadata() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("folder")).unwrap();
        fs::write(root.path().join("folder/source.md"), "source\n").unwrap();
        fs::write(root.path().join("target.md"), "target\n").unwrap();
        let manager = crate::crdt::CrdtManager::new();
        crate::catalog_sync::prepare(root.path(), &manager, "a").unwrap();
        apply_operations(root.path(), &[LinkOperation {
            source: "folder/source.md".into(), target: "target.md".into(), action: LinkAction::add,
        }], LinkOrigin::manual).unwrap();
        let old_history = fs::read(operations_path(root.path())).unwrap();
        let old_projection = fs::read(links_path(root.path())).unwrap();
        crate::structural::rename(root.path(), "folder", "moved", &manager, "a").unwrap();
        crate::structural::rename(root.path(), "target.md", "new.md", &manager, "a").unwrap();
        assert_eq!(graph_links(root.path()).unwrap(), vec![LinkEdge {
            source: "moved/source.md".into(), target: "new.md".into(), origin: LinkOrigin::manual,
        }]);
        apply_operations(root.path(), &[LinkOperation {
            source: "moved/source.md".into(), target: "new.md".into(), action: LinkAction::remove,
        }], LinkOrigin::manual).unwrap();
        merge_sync(root.path(), crate::link_operations::RELATIVE_PATH, &old_history).unwrap();
        merge_sync(root.path(), LINKS_REL_PATH, &old_projection).unwrap();
        assert!(graph_links(root.path()).unwrap().is_empty());
        apply_operations(root.path(), &[LinkOperation {
            source: "moved/source.md".into(), target: "new.md".into(), action: LinkAction::add,
        }], LinkOrigin::agent).unwrap();
        assert_eq!(graph_links(root.path()).unwrap().len(), 1);
    }

    #[test]
    fn a_link_to_a_deleted_identity_never_attaches_to_a_reused_name_and_can_be_restored() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("source.md"), "source").unwrap();
        fs::write(root.path().join("target.md"), "target").unwrap();
        let manager = crate::crdt::CrdtManager::new();
        crate::catalog_sync::prepare(root.path(), &manager, "local").unwrap();
        apply_operations(root.path(), &[LinkOperation {
            source: "source.md".into(), target: "target.md".into(), action: LinkAction::add,
        }], LinkOrigin::manual).unwrap();
        crate::catalog_sync::delete(root.path(), "target.md", &manager, "local").unwrap();
        crate::vault::create_note(root.path(), "target.md", Some("replacement"), "en-US").unwrap();
        assert!(graph_links(root.path()).unwrap().is_empty());
        apply_operations(root.path(), &[LinkOperation {
            source: "source.md".into(), target: "target.md".into(), action: LinkAction::add,
        }], LinkOrigin::agent).unwrap();
        assert_eq!(graph_links(root.path()).unwrap(), vec![LinkEdge {
            source: "source.md".into(), target: "target.md".into(), origin: LinkOrigin::agent,
        }]);
        let restored = crate::catalog_sync::restore_latest(root.path(), &manager, "local").unwrap().unwrap();
        let links = graph_links(root.path()).unwrap();
        assert_eq!(links.len(), 2);
        assert!(links.iter().any(|edge| edge.target == restored.0 && edge.origin == LinkOrigin::manual));
        assert_ne!(restored.0, "target.md");
        assert_eq!(crate::vault::read_note(root.path(), "target.md").unwrap(), "replacement");
    }

    #[test]
    fn endpoint_bindings_preserve_addition_payloads_and_reject_rebinding_without_writing() {
        let root = tempfile::tempdir().unwrap();
        for name in ["a.md", "b.md", "c.md"] { fs::write(root.path().join(name), name).unwrap(); }
        let manager = crate::crdt::CrdtManager::new();
        let catalog = crate::catalog_sync::prepare(root.path(), &manager, "local").unwrap();
        apply_operations(root.path(), &[LinkOperation {
            source: "a.md".into(), target: "b.md".into(), action: LinkAction::add,
        }], LinkOrigin::manual).unwrap();
        let original = LinkChanges::decode(&fs::read(operations_path(root.path())).unwrap()).unwrap();
        crate::structural::rename(root.path(), "b.md", "moved.md", &manager, "local").unwrap();
        let bytes = fs::read(operations_path(root.path())).unwrap();
        let renamed = LinkChanges::decode(&bytes).unwrap();
        assert_eq!(original.additions, renamed.additions);
        assert_eq!(original.identities, renamed.identities);
        let c = catalog.resolve().unwrap().values().find(|entry| entry.path == "c.md").unwrap().id.clone();
        let mut invalid = renamed;
        invalid.identities.values_mut().next().unwrap().target = c;
        assert!(merge_sync(root.path(), crate::link_operations::RELATIVE_PATH, &invalid.encode().unwrap()).is_err());
        assert_eq!(fs::read(operations_path(root.path())).unwrap(), bytes);
        assert_eq!(graph_links(root.path()).unwrap()[0].target, "moved.md");
    }
}
