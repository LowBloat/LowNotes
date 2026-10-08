use crate::{
    catalog_sync,
    commands::{catalog_device, AppState},
    crdt::CrdtManager,
    note_history, retention, structural, vault,
};
use anyhow::{bail, Context};
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, State};

struct ContextData {
    root: PathBuf,
    manager: CrdtManager,
    device: String,
    peers: Vec<String>,
}
fn context(state: &AppState, vault_id: &str) -> Result<ContextData, String> {
    let settings = state.settings.read();
    let vault = settings.active_vault().ok_or("errors.noActiveVault")?;
    if vault.id != vault_id {
        return Err("ai.vaultChanged".into());
    }
    Ok(ContextData {
        root: vault.path.clone(),
        manager: state.crdt.clone(),
        device: catalog_device(state, vault),
        peers: vault
            .peers
            .iter()
            .map(|peer| peer.endpoint_id.clone())
            .collect(),
    })
}
async fn worker<T: Send + 'static>(
    task: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}
#[derive(Debug, Serialize)]
pub struct NoteSnapshot {
    pub note_id: String,
    pub path: String,
    pub content: String,
    pub hash: String,
}
#[derive(Serialize)]
pub struct HistoryListing {
    pub note: Option<NoteSnapshot>,
    pub versions: Vec<note_history::VersionSummary>,
    pub trash: Vec<catalog_sync::TrashEntry>,
    pub retention: retention::RetentionPolicy,
}
#[tauri::command]
pub async fn history_list(
    vault_id: String,
    path: Option<String>,
    state: State<'_, AppState>,
) -> Result<HistoryListing, String> {
    let data = context(&state, &vault_id)?;
    worker(move || {
        structural::exclusive(&data.root, &data.manager, || {
            let catalog = catalog_sync::prepare(&data.root, &data.manager, &data.device)?;
            let entry = catalog.resolve()?.into_values().find(|entry| {
                !entry.deleted() && !entry.is_dir && Some(&entry.path) == path.as_ref()
            });
            let note = entry
                .map(|entry| -> anyhow::Result<_> {
                    data.manager.get_or_create_doc(&data.root, &entry.path)?;
                    let content = vault::read_note(&data.root, &entry.path)?;
                    Ok(NoteSnapshot {
                        note_id: entry.id,
                        path: entry.path,
                        hash: blake3::hash(content.as_bytes()).to_hex().to_string(),
                        content,
                    })
                })
                .transpose()?;
            let versions = note
                .as_ref()
                .map(|note| note_history::list(&data.root, &note.note_id))
                .transpose()?
                .unwrap_or_default();
            Ok(HistoryListing {
                note,
                versions,
                trash: catalog_sync::list_trash(&data.root)?,
                retention: retention::load(&data.root)?,
            })
        })
    })
    .await
}
#[tauri::command]
pub async fn history_version(
    vault_id: String,
    note_id: String,
    version_id: String,
    state: State<'_, AppState>,
) -> Result<note_history::VersionContent, String> {
    let data = context(&state, &vault_id)?;
    worker(move || note_history::get(&data.root, &note_id, &version_id)).await
}
#[tauri::command]
pub async fn history_apply(
    vault_id: String,
    note_id: String,
    expected_hash: String,
    version_id: Option<String>,
    content: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<String, String> {
    let data = context(&state, &vault_id)?;
    let (path, result) = worker(move || match (version_id, content) {
        (Some(version), None) => note_history::restore(
            &data.root,
            &note_id,
            &version,
            &expected_hash,
            &data.manager,
        ),
        (None, Some(content)) => note_history::apply(
            &data.root,
            &note_id,
            &content,
            &expected_hash,
            &data.manager,
        ),
        _ => {
            bail!("invalid history action");
        }
    })
    .await?;
    if result.changed {
        let _ = app.emit(
            "p2p:crdt-update",
            serde_json::json!({ "type": "RemoteCrdtUpdate", "note_path": path,
            "update": result.state, "vault_id": vault_id }),
        );
        let settings = state.settings.read();
        if settings
            .active_vault()
            .is_some_and(|vault| vault.id == vault_id)
        {
            if let Some(net) = state.network.read().as_ref() {
                net.broadcast_crdt_update(path.clone(), result.state);
                net.sync_now();
            }
        }
    }
    Ok(path)
}
#[tauri::command]
pub async fn history_trash_read(
    vault_id: String,
    record_id: String,
    note_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let data = context(&state, &vault_id)?;
    worker(move || {
        structural::exclusive(&data.root, &data.manager, || {
            catalog_sync::read_trash_note(&data.root, &record_id, &note_id)
        })
    })
    .await
}
#[derive(Serialize)]
pub struct TrashRestored {
    pub path: String,
    pub is_dir: bool,
}
#[tauri::command]
pub async fn history_trash_restore(
    vault_id: String,
    record_id: String,
    note_id: String,
    state: State<'_, AppState>,
) -> Result<TrashRestored, String> {
    let data = context(&state, &vault_id)?;
    let result = worker(move || {
        let (path, is_dir) = catalog_sync::restore_selected(
            &data.root,
            &record_id,
            &note_id,
            &data.manager,
            &data.device,
        )?;
        Ok(TrashRestored { path, is_dir })
    })
    .await?;
    if state
        .settings
        .read()
        .active_vault()
        .is_some_and(|vault| vault.id == vault_id)
    {
        if let Some(net) = state.network.read().as_ref() {
            net.sync_now();
        }
    }
    Ok(result)
}
#[tauri::command]
pub async fn history_retention_save(
    vault_id: String,
    policy: retention::RetentionPolicy,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let data = context(&state, &vault_id)?;
    worker(move || {
        structural::exclusive(&data.root, &data.manager, || {
            retention::save(&data.root, policy)
        })
    })
    .await
}
#[tauri::command]
pub async fn history_cleanup(
    vault_id: String,
    apply: bool,
    state: State<'_, AppState>,
) -> Result<retention::CleanupReport, String> {
    let data = context(&state, &vault_id)?;
    worker(move || retention::cleanup(&data.root, &data.peers, &data.manager, apply)).await
}

pub fn start_retention(
    settings: std::sync::Arc<parking_lot::RwLock<crate::config::AppSettings>>,
    manager: CrdtManager,
) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15 * 60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let vault = settings.read().active_vault().cloned();
            let Some(vault) = vault else {
                continue;
            };
            let root = vault.path.clone();
            let peers: Vec<_> = vault
                .peers
                .into_iter()
                .map(|peer| peer.endpoint_id)
                .collect();
            let manager = manager.clone();
            if let Err(error) = worker(move || {
                retention::cleanup(&root, &peers, &manager, true)
                    .context("recovery retention failed")
            })
            .await
            {
                crate::storage::report_recovery(
                    &vault.path.join(".lownotes/history-settings.json"),
                    false,
                );
                eprintln!("history retention: {error}");
            }
        }
    });
}
