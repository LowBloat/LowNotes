//! Vault operations shared by the Tauri commands and the MCP server. Reads take
//! the vault their caller resolved, so a vault switch in between cannot serve
//! another one; changes perform the side effects that must follow them (editor
//! event, peer broadcast, sync). Callers run them on the blocking pool, never
//! on the main thread.
use tauri::{AppHandle, Emitter};

use crate::{
    assistant::{self, NoteDraft, NoteEdit},
    commands::{catalog_device, AppState},
    config::VaultConfig,
    links::{self, LinkEdge},
    network::NetworkEventPayload,
    rag::{self, RagChunk},
    vault::{self, VaultItem},
};

/// A note's Markdown, its collaborative state and its stable identity.
pub struct NoteRead {
    pub content: String,
    pub crdt_state: Vec<u8>,
    pub note_id: Option<String>,
}

pub fn active_vault(state: &AppState) -> Result<VaultConfig, String> {
    state
        .settings
        .read()
        .active_vault()
        .cloned()
        .ok_or_else(|| "errors.noActiveVault".to_string())
}

/// The active vault, only while it is still the one `vault_id` names.
fn expected_vault(state: &AppState, vault_id: &str) -> Result<VaultConfig, String> {
    let vault = active_vault(state)?;
    if vault.id != vault_id {
        return Err("ai.vaultChanged".into());
    }
    Ok(vault)
}

pub fn list_notes(vault: &VaultConfig) -> Result<Vec<VaultItem>, String> {
    vault::list_vault_items(&vault.path).map_err(|e| e.to_string())
}

pub fn read_note(state: &AppState, vault: &VaultConfig, path: &str) -> Result<NoteRead, String> {
    let catalog =
        crate::catalog_sync::prepare(&vault.path, &state.crdt, &catalog_device(state, vault))
            .map_err(|e| e.to_string())?;
    let note_id = catalog
        .resolve()
        .map_err(|e| e.to_string())?
        .into_values()
        .find(|entry| !entry.deleted() && !entry.is_dir && entry.path == path)
        .map(|entry| entry.id);
    vault::read_note(&vault.path, path).map_err(|e| e.to_string())?;

    let crdt_state = state
        .crdt
        .get_or_create_doc(&vault.path, path)
        .map_err(|e| e.to_string())?;
    let content = vault::read_note(&vault.path, path).map_err(|e| e.to_string())?;
    Ok(NoteRead {
        content,
        crdt_state,
        note_id,
    })
}

pub fn search(vault: &VaultConfig, query: &str, limit: usize) -> Result<Vec<RagChunk>, String> {
    rag::index_and_search_vault(&vault.path, query, limit).map_err(|e| e.to_string())
}

pub fn links(vault: &VaultConfig) -> Result<Vec<LinkEdge>, String> {
    links::graph_links(&vault.path).map_err(|e| e.to_string())
}

/// The links that point to `path`.
pub fn backlinks(vault: &VaultConfig, path: &str) -> Result<Vec<LinkEdge>, String> {
    Ok(edges_to(links(vault)?, path))
}

fn edges_to(edges: Vec<LinkEdge>, path: &str) -> Vec<LinkEdge> {
    edges
        .into_iter()
        .filter(|edge| edge.target == path)
        .collect()
}

/// Creates a note from an assistant draft. A draft never overwrites a note.
pub fn save_draft(state: &AppState, vault_id: &str, draft: &NoteDraft) -> Result<String, String> {
    let vault = expected_vault(state, vault_id)?;
    let path = assistant::save_draft(&vault.path, draft).map_err(|e| e.to_string())?;
    state
        .crdt
        .invalidate_path(&vault.path, &path)
        .map_err(|e| e.to_string())?;
    if let Err(e) = links::reconcile_wikilinks(&vault.path, &path, &draft.content) {
        tracing::warn!("reconcile_wikilinks failed for {path}: {e}");
    }
    if let Some(net) = state.network.read().as_ref() {
        net.sync_now();
    }
    Ok(path)
}

/// Applies an exact text edit through the collaborative state, so the open
/// editor and the paired devices receive it like any other change.
pub fn apply_edit(
    state: &AppState,
    app: &AppHandle,
    vault_id: &str,
    edit: &NoteEdit,
) -> Result<String, String> {
    let vault = expected_vault(state, vault_id)?;
    let result = state
        .crdt
        .apply_note_edit(&vault.path, edit)
        .map_err(|e| e.to_string())?;
    // The open editor receives the same CRDT state as the paired devices.
    let _ = app.emit(
        "p2p:crdt-update",
        NetworkEventPayload::RemoteCrdtUpdate {
            note_path: edit.path.clone(),
            update: result.state.clone(),
        },
    );
    if let Some(net) = state.network.read().as_ref() {
        net.broadcast_crdt_update(edit.path.clone(), result.state);
        net.sync_now();
    }
    Ok(edit.path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::LinkOrigin;

    fn edge(source: &str, target: &str) -> LinkEdge {
        LinkEdge {
            source: source.into(),
            target: target.into(),
            origin: LinkOrigin::wikilink,
        }
    }

    #[test]
    fn backlinks_keep_only_edges_that_point_to_the_note() {
        let edges = vec![
            edge("a.md", "b.md"),
            edge("b.md", "a.md"),
            edge("c.md", "b.md"),
            edge("c.md", "folder/b.md"),
        ];
        let sources: Vec<String> = edges_to(edges, "b.md")
            .into_iter()
            .map(|edge| edge.source)
            .collect();
        assert_eq!(sources, ["a.md", "c.md"]);
    }
}
