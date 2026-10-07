use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

pub const MAX_NOTE_BYTES: u64 = 10 * 1024 * 1024; // 10MB

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteMeta {
    pub path: String,
    pub modified_ms: u64,
    pub size: u64,
    pub hash: String,
}

pub type Manifest = BTreeMap<String, NoteMeta>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultItem {
    pub path: String,
    pub name: String,
    pub title: String,
    pub modified_ms: u64,
    pub size: u64,
    pub is_dir: bool,
}

pub fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("md" | "markdown")
    )
}

pub fn safe_join(root: &Path, relative_wire: &str) -> anyhow::Result<PathBuf> {
    let clean = relative_wire.replace('\\', "/");
    let candidate = Path::new(&clean);
    let mut resolved = root.to_path_buf();

    for component in candidate.components() {
        match component {
            Component::Normal(segment) => {
                let name = segment.to_str().context("errors.invalidPathChars")?;
                if name.starts_with('.') && name != ".lownotes" {
                    bail!("errors.hiddenPath");
                }
                resolved.push(segment);
            }
            Component::CurDir => {}
            _ => bail!("errors.pathEscape"),
        }
    }

    Ok(resolved)
}

pub fn list_vault_items(root: &Path) -> anyhow::Result<Vec<VaultItem>> {
    crate::note_transaction::recover_all(root)?;
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let mut items = Vec::new();
    let walker = WalkDir::new(root)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            let name = entry.file_name().to_str().unwrap_or("");
            !name.starts_with('.') && name != "node_modules" && name != "target"
        });

    for entry_res in walker {
        let entry = match entry_res {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        let is_dir = entry.file_type().is_dir();

        if !is_dir && !is_markdown(path) {
            continue;
        }

        let relative = match path.strip_prefix(root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let metadata = entry.metadata().ok();
        let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified_ms = metadata
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let title = if is_dir {
            file_name.clone()
        } else {
            extract_title_or_name(path, &file_name)
        };

        items.push(VaultItem {
            path: relative,
            name: file_name,
            title,
            modified_ms,
            size,
            is_dir,
        });
    }

    // Sort: folders first, then alphabetical by title
    items.sort_by(|a, b| {
        match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.path.to_lowercase().cmp(&b.path.to_lowercase()),
        }
    });

    Ok(items)
}

fn extract_title_or_name(path: &Path, file_name: &str) -> String {
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines().take(10) {
            let trimmed = line.trim();
            if let Some(title) = trimmed.strip_prefix("# ") {
                let clean = title.trim();
                if !clean.is_empty() {
                    return clean.to_string();
                }
            }
        }
    }
    file_name.strip_suffix(".md").or_else(|| file_name.strip_suffix(".markdown")).unwrap_or(file_name).to_string()
}

pub fn read_note(root: &Path, relative: &str) -> anyhow::Result<String> {
    let target = safe_join(root, relative)?;
    let metadata = fs::metadata(&target).context("errors.noteNotFound")?;
    if metadata.len() > MAX_NOTE_BYTES {
        bail!("errors.noteTooLarge");
    }
    fs::read_to_string(target).context("errors.noteReadFail")
}

pub fn save_note(root: &Path, relative: &str, content: &str) -> anyhow::Result<()> {
    let target = safe_join(root, relative)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::storage::write_text(&target, content)?;
    Ok(())
}

pub fn create_note(
    root: &Path,
    relative: &str,
    initial_content: Option<&str>,
    lang: &str,
) -> anyhow::Result<String> {
    let mut clean_relative = relative.trim().replace('\\', "/");
    if !clean_relative.ends_with(".md") && !clean_relative.ends_with(".markdown") {
        clean_relative.push_str(".md");
    }

    let target = safe_join(root, &clean_relative)?;
    if target.exists() {
        bail!("errors.noteExists");
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }

    let default_content = initial_content
        .map(|s| s.to_string())
        .unwrap_or_else(|| default_note_content(lang));
    crate::storage::write_text(&target, &default_content)?;
    Ok(clean_relative)
}

/// Template written into brand-new notes when no initial content is supplied.
/// Mirrors the `note.default*` UI copy per supported language.
fn default_note_content(lang: &str) -> String {
    match lang {
        "pt-BR" => "# Nova Nota\n\nComece a escrever aqui...\n".to_string(),
        "es-ES" => "# Nueva Nota\n\nComienza a escribir aquí...\n".to_string(),
        _ => "# New Note\n\nStart writing here...\n".to_string(),
    }
}

pub fn create_folder(root: &Path, relative: &str) -> anyhow::Result<()> {
    let target = safe_join(root, relative)?;
    fs::create_dir_all(&target)?;
    Ok(())
}

pub fn rename_item(root: &Path, old_relative: &str, new_relative: &str) -> anyhow::Result<()> {
    crate::note_transaction::recover_all(root)?;
    let source = safe_join(root, old_relative)?;
    let destination = safe_join(root, new_relative)?;
    if !source.exists() {
        bail!("errors.sourceNotFound");
    }
    if destination.exists() {
        bail!("errors.targetExists");
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(source, destination)?;
    Ok(())
}

pub fn delete_item(root: &Path, relative: &str) -> anyhow::Result<()> {
    crate::note_transaction::recover_all(root)?;
    let target = safe_join(root, relative)?;
    if !target.exists() {
        return Ok(());
    }
    if target.is_dir() {
        fs::remove_dir_all(target)?;
    } else {
        fs::remove_file(target)?;
    }
    Ok(())
}

pub fn build_manifest(root: &Path) -> anyhow::Result<Manifest> {
    crate::note_transaction::recover_all(root)?;
    crate::links::prepare_sync(root)?;
    let mut manifest = Manifest::new();
    let items = list_vault_items(root)?;

    for item in items {
        if item.is_dir {
            continue;
        }
        let target = safe_join(root, &item.path)?;
        if let Ok(bytes) = fs::read(&target) {
            let hash = blake3::hash(&bytes).to_hex().to_string();
            manifest.insert(
                item.path.clone(),
                NoteMeta {
                    path: item.path,
                    modified_ms: item.modified_ms,
                    size: item.size,
                    hash,
                },
            );
        }
    }

    // Include the hidden links store so P2P sync (manifest-driven) propagates it.
    for links_rel in [crate::links::LINKS_REL_PATH, crate::link_operations::RELATIVE_PATH] {
        let links_file = root.join(links_rel);
        if links_file.is_file() {
            if let (Ok(bytes), Ok(metadata)) = (fs::read(&links_file), fs::metadata(&links_file)) {
                let hash = blake3::hash(&bytes).to_hex().to_string();
                let modified_ms = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                manifest.insert(
                    links_rel.to_string(),
                    NoteMeta {
                        path: links_rel.to_string(),
                        modified_ms,
                        size: metadata.len(),
                        hash,
                    },
                );
            }
        }
    }

    // CRDT history is synced separately from Markdown content and merged on receipt.
    let state_dir = root.join(".lownotes/crdt");
    if state_dir.is_dir() {
        for entry in WalkDir::new(&state_dir).min_depth(1).max_depth(1).into_iter().filter_map(Result::ok) {
            if !entry.file_type().is_file() || entry.path().extension().and_then(|e| e.to_str()) != Some("bin") {
                continue;
            }
            let relative = entry.path().strip_prefix(root)?.to_string_lossy().replace('\\', "/");
            let bytes = crate::crdt::CrdtManager::read_state_file(root, &relative)?.context("CRDT state missing")?;
            let metadata = entry.metadata()?;
            let modified_ms = metadata.modified().ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as u64).unwrap_or(0);
            manifest.insert(relative.clone(), NoteMeta {
                path: relative, modified_ms, size: metadata.len(),
                hash: blake3::hash(&bytes).to_hex().to_string(),
            });
        }
    }

    manifest.extend(crate::local_images::manifest(root)?);
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_join_prevents_escape() {
        let root = Path::new("C:/test_vault");
        assert!(safe_join(root, "notes/sub/doc.md").is_ok());
        assert!(safe_join(root, "../outside.md").is_err());
        assert!(safe_join(root, "notes/../../outside.md").is_err());
        assert!(safe_join(root, ".hidden/file.md").is_err());
    }

    #[test]
    fn test_is_markdown() {
        assert!(is_markdown(Path::new("note.md")));
        assert!(is_markdown(Path::new("note.markdown")));
        assert!(!is_markdown(Path::new("note.txt")));
        assert!(!is_markdown(Path::new("image.png")));
    }
}
