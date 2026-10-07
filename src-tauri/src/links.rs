use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::bail;
use serde::{Deserialize, Serialize};

use crate::vault;

pub const LINKS_VERSION: u8 = 1;
pub const LINKS_REL_PATH: &str = ".lownotes/links.json";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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

/// Load the link store. Missing or corrupted file yields an empty store (version 1).
pub fn load_links(vault: &Path) -> LinkStore {
    crate::storage::read_validated(&links_path(vault), valid_store)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_slice::<LinkStore>(&raw).ok())
        .unwrap_or_default()
}

fn valid_store(bytes: &[u8]) -> bool {
    serde_json::from_slice::<LinkStore>(bytes).is_ok_and(|store| store.version == LINKS_VERSION)
}

pub fn save_links(vault: &Path, store: &LinkStore) -> anyhow::Result<()> {
    let path = links_path(vault);
    crate::storage::read_validated(&path, valid_store)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(store)?;
    crate::storage::write_validated(&path, json.as_bytes(), valid_store)?;
    Ok(())
}

/// Scan `[[...]]` tokens, skipping fenced code blocks and inline code spans.
/// Returns `(token, span_start, span_end)` with byte offsets into `content`.
fn wikilink_spans(content: &str) -> Vec<(String, usize, usize)> {
    let mut spans = Vec::new();
    let mut in_fence = false;
    let mut line_offset = 0usize;

    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            line_offset += line.len();
            continue;
        }
        if in_fence {
            line_offset += line.len();
            continue;
        }

        let b = line.as_bytes();
        let mut i = 0usize;
        while i < b.len() {
            if b[i] == b'`' {
                // Inline code span: opening backtick run closes on a run of equal length.
                let mut run = 0usize;
                let mut j = i;
                while j < b.len() && b[j] == b'`' {
                    run += 1;
                    j += 1;
                }
                let mut k = j;
                let mut closed_at = None;
                while k < b.len() {
                    if b[k] == b'`' {
                        let mut run2 = 0usize;
                        while k < b.len() && b[k] == b'`' {
                            run2 += 1;
                            k += 1;
                        }
                        if run2 == run {
                            closed_at = Some(k);
                            break;
                        }
                    } else {
                        k += 1;
                    }
                }
                // Unterminated backtick run: treat the rest of the line as code.
                i = closed_at.unwrap_or(b.len());
                continue;
            }

            if b[i] == b'[' && i + 1 < b.len() && b[i + 1] == b'[' {
                let inner_start = i + 2;
                let mut j = inner_start;
                let mut end = None;
                while j + 1 < b.len() {
                    if b[j] == b']' && b[j + 1] == b']' {
                        end = Some(j);
                        break;
                    }
                    j += 1;
                }
                if let Some(end) = end {
                    let token = line[inner_start..end].trim().to_string();
                    if !token.is_empty() {
                        spans.push((token, line_offset + i, line_offset + end + 2));
                    }
                    i = end + 2;
                    continue;
                }
                i += 2;
                continue;
            }

            i += 1;
        }

        line_offset += line.len();
    }

    spans
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
    let mut spans = Vec::new();
    let mut in_fence = false;
    let mut offset = 0;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            offset += line.len();
            continue;
        }
        if in_fence {
            offset += line.len();
            continue;
        }
        let b = line.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'`' {
                let run = b[i..].iter().take_while(|&&c| c == b'`').count();
                i += run;
                while i < b.len() {
                    if b[i..].starts_with(&vec![b'`'; run]) {
                        i += run;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
            if b[i] == b'[' && (i == 0 || b[i - 1] != b'!' && b[i - 1] != b'[') {
                if let Some(label_end) = b[i + 1..]
                    .iter()
                    .position(|&c| c == b']')
                    .map(|p| i + 1 + p)
                {
                    if b.get(label_end + 1) == Some(&b'(') {
                        if let Some(dest_end) = b[label_end + 2..]
                            .iter()
                            .position(|&c| c == b')')
                            .map(|p| label_end + 2 + p)
                        {
                            let dest = line[label_end + 2..dest_end]
                                .trim()
                                .trim_matches(['<', '>']);
                            if !dest.contains("://")
                                && !dest.starts_with('#')
                                && [".md", ".markdown"].iter().any(|ext| {
                                    dest.split('#')
                                        .next()
                                        .unwrap_or("")
                                        .to_ascii_lowercase()
                                        .ends_with(ext)
                                })
                            {
                                spans.push((
                                    dest.to_string(),
                                    offset + i,
                                    offset + dest_end + 1,
                                    line[i + 1..label_end].to_string(),
                                ));
                            }
                            i = dest_end + 1;
                            continue;
                        }
                    }
                }
            }
            i += 1;
        }
        offset += line.len();
    }
    spans
}

fn note_tokens(content: &str) -> Vec<String> {
    extract_wikilinks(content)
        .into_iter()
        .chain(
            markdown_link_spans(content)
                .into_iter()
                .map(|(token, _, _, _)| token),
        )
        .collect()
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
        .split('#')
        .next()?
        .trim()
        .trim_matches(['<', '>'])
        .replace('\\', "/")
        .replace("%20", " ");
    if clean.is_empty() || clean.contains("://") {
        return None;
    }
    let parent = source.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
    let mut candidates = Vec::new();
    if clean.starts_with("./") || clean.starts_with("../") {
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
    let mut edges: Vec<LinkEdge> = load_links(vault)
        .links
        .into_iter()
        .filter(|e| {
            e.origin != LinkOrigin::wikilink
                && exists(&e.source)
                && exists(&e.target)
                && e.source != e.target
        })
        .collect();
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
    let mut store = load_links(vault);
    let before = store.links.clone();

    store
        .links
        .retain(|e| !(e.origin == LinkOrigin::wikilink && e.source == source));

    let items = vault::list_vault_items(vault)?;
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
        save_links(vault, &store)?;
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
fn strip_links_to(vault: &Path, source: &str, content: &str, target: &str) -> Option<String> {
    let items = vault::list_vault_items(vault).ok()?;
    let mut replacements: Vec<(usize, usize, String)> = wikilink_spans(content)
        .into_iter()
        .filter(|(token, _, _)| {
            resolve_from_items(&items, source, token).as_deref() == Some(target)
        })
        .map(|(token, start, end)| {
            let label = if let Some((_, alias)) = token.split_once('|') {
                alias.to_string()
            } else {
                token
                    .split('#')
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
                resolve_from_items(&items, source, token).as_deref() == Some(target)
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
    for op in ops {
        validate_note(vault, &op.source)?;
        validate_note(vault, &op.target)?;
    }

    let mut store = load_links(vault);
    let mut wikilink_removals: Vec<(String, String)> = Vec::new();

    for op in ops {
        match op.action {
            LinkAction::add => {
                let exists = store
                    .links
                    .iter()
                    .any(|e| e.source == op.source && e.target == op.target);
                if !exists {
                    store.links.push(LinkEdge {
                        source: op.source.clone(),
                        target: op.target.clone(),
                        origin: origin.clone(),
                    });
                }
            }
            LinkAction::remove => {
                store
                    .links
                    .retain(|e| !(e.source == op.source && e.target == op.target));
                wikilink_removals.push((op.source.clone(), op.target.clone()));
            }
        }
    }

    save_links(vault, &store)?;

    for (source, target) in &wikilink_removals {
        let Ok(content) = vault::read_note(vault, source) else {
            continue;
        };
        let Some(updated) = strip_links_to(vault, source, &content, target) else {
            continue;
        };
        if let Err(e) = vault::save_note(vault, source, &updated) {
            eprintln!("apply_operations: failed to strip wikilink in {source}: {e}");
            continue;
        }
        if let Err(e) = reconcile_wikilinks(vault, source, &updated) {
            eprintln!("apply_operations: reconcile failed for {source}: {e}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_links_recover_valid_relationships_and_cannot_overwrite_unrecoverable_data() {
        let root = tempfile::tempdir().unwrap();
        let mut store = LinkStore::new();
        store.links.push(LinkEdge { source: "a.md".into(), target: "b.md".into(), origin: LinkOrigin::manual });
        save_links(root.path(), &store).unwrap();
        let expected = store.clone();
        store.links.push(LinkEdge { source: "b.md".into(), target: "c.md".into(), origin: LinkOrigin::agent });
        save_links(root.path(), &store).unwrap();
        fs::write(links_path(root.path()), b"truncated").unwrap();
        assert_eq!(load_links(root.path()), expected);
        fs::write(links_path(root.path()), b"broken again").unwrap();
        fs::remove_file(crate::storage::backup_path(&links_path(root.path()))).unwrap();
        assert!(save_links(root.path(), &LinkStore::new()).is_err());
        assert_eq!(fs::read(links_path(root.path())).unwrap(), b"broken again");
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

        // Corrupted file => empty store, version 1.
        fs::write(links_path(&vault), "{ not json").unwrap();
        let corrupt = load_links(&vault);
        assert_eq!(corrupt.version, 1);
        assert!(corrupt.links.is_empty());

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
}
