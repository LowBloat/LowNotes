use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::{
    assistant::{self, AssistantSkill, NoteDraft, NoteEdit},
    chat_history::{self, ChatHistory},
    config::{AiProviderConfig, AiSettings, AppSettings, BUILTIN_PALETTE_IDS, ThemePalettesSettings, VaultConfig, WebSearchSettings, decode_pair_code},
    crdt::CrdtManager,
    links,
    network::{NetworkIdentity, NetworkService, PairInfo},
    rag::{self, ChatMessage, ChatResponse, RagChunk},
    undo::{DeletedSnapshot, RestoredItem, UndoHistory},
    vault::{self, VaultItem},
    web_search,
};

pub struct AppState {
    pub settings: Arc<RwLock<AppSettings>>,
    pub crdt: CrdtManager,
    pub network: Arc<RwLock<Option<NetworkService>>>,
    pub undo: Mutex<UndoHistory>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InitialStateResponse {
    pub settings: AppSettings,
    pub active_vault: Option<VaultConfig>,
    pub items: Vec<VaultItem>,
    pub pair_info: Option<PairInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NoteReadResponse {
    pub content: String,
    pub crdt_update_base64: String,
}

#[tauri::command]
pub fn get_app_state(state: State<'_, AppState>) -> Result<InitialStateResponse, String> {
    let settings = state.settings.read().clone();
    let active_vault = settings.active_vault().cloned();
    let items = if let Some(vault) = &active_vault {
        vault::list_vault_items(&vault.path).map_err(|e| e.to_string())?
    } else {
        Vec::new()
    };

    let pair_info = state.network.read().as_ref().and_then(|s| s.pair_info());

    Ok(InitialStateResponse {
        settings,
        active_vault,
        items,
        pair_info,
    })
}

#[tauri::command]
pub fn select_vault(
    path_str: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<InitialStateResponse, String> {
    let path = PathBuf::from(path_str);
    if !path.is_dir() {
        return Err("errors.invalidFolderPath".to_string());
    }

    let mut settings = state.settings.write();
    let vault_id = if let Some(existing) = settings.vaults.iter().find(|v| v.path == path) {
        existing.id.clone()
    } else {
        let new_vault = VaultConfig::new(path.clone(), None);
        let id = new_vault.id.clone();
        settings.vaults.push(new_vault);
        id
    };

    settings.active_vault_id = Some(vault_id);
    settings.save().map_err(|e| e.to_string())?;

    let active_vault = settings.active_vault().cloned().ok_or("errors.vaultNotFound")?;
    drop(settings);

    // Restart network service for the selected vault
    restart_network_service(&state, &active_vault, app)?;

    let items = vault::list_vault_items(&active_vault.path).map_err(|e| e.to_string())?;
    let current_settings = state.settings.read().clone();

    let pair_info = state.network.read().as_ref().and_then(|s| s.pair_info());

    Ok(InitialStateResponse {
        settings: current_settings,
        active_vault: Some(active_vault),
        items,
        pair_info,
    })
}

#[tauri::command]
pub fn create_vault(
    path_str: String,
    name: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<InitialStateResponse, String> {
    let path = PathBuf::from(path_str);
    std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;

    let mut settings = state.settings.write();
    let new_vault = VaultConfig::new(path.clone(), name);
    let id = new_vault.id.clone();
    settings.vaults.push(new_vault);
    settings.active_vault_id = Some(id);
    settings.save().map_err(|e| e.to_string())?;

    let active_vault = settings.active_vault().cloned().ok_or("errors.vaultNotFound")?;
    drop(settings);

    restart_network_service(&state, &active_vault, app)?;

    let items = vault::list_vault_items(&active_vault.path).map_err(|e| e.to_string())?;
    let current_settings = state.settings.read().clone();

    let pair_info = state.network.read().as_ref().and_then(|s| s.pair_info());

    Ok(InitialStateResponse {
        settings: current_settings,
        active_vault: Some(active_vault),
        items,
        pair_info,
    })
}

#[tauri::command]
pub fn list_notes(state: State<'_, AppState>) -> Result<Vec<VaultItem>, String> {
    let settings = state.settings.read();
    let vault = settings.active_vault().ok_or("errors.noActiveVault")?;
    vault::list_vault_items(&vault.path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn read_note(path: String, state: State<'_, AppState>) -> Result<NoteReadResponse, String> {
    let settings = state.settings.read();
    let vault = settings.active_vault().ok_or("errors.noActiveVault")?;
    vault::read_note(&vault.path, &path).map_err(|e| e.to_string())?;

    let crdt_bytes = state.crdt.get_or_create_doc(&vault.path, &path).map_err(|e| e.to_string())?;
    let content = vault::read_note(&vault.path, &path).map_err(|e| e.to_string())?;
    let crdt_update_base64 = URL_SAFE_NO_PAD.encode(crdt_bytes);

    Ok(NoteReadResponse {
        content,
        crdt_update_base64,
    })
}

#[tauri::command]
pub fn save_note(path: String, content: String, state: State<'_, AppState>) -> Result<(), String> {
    let settings = state.settings.read();
    let vault = settings.active_vault().ok_or("errors.noActiveVault")?;
    vault::save_note(&vault.path, &path, &content).map_err(|e| e.to_string())?;
    state.crdt.remove_doc(&vault.path, &path).map_err(|e| e.to_string())?;
    if let Err(e) = links::reconcile_wikilinks(&vault.path, &path, &content) {
        eprintln!("reconcile_wikilinks failed for {path}: {e}");
    }
    Ok(())
}

#[tauri::command]
pub fn create_note(
    path: String,
    title: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let settings = state.settings.read();
    let vault = settings.active_vault().ok_or("errors.noActiveVault")?;
    let lang = settings.language.clone();
    let init_content = title.map(|t| format!("# {t}\n\n"));
    let created = vault::create_note(&vault.path, &path, init_content.as_deref(), &lang)
        .map_err(|e| e.to_string())?;

    // Best-effort AI auto-linking for brand-new notes.
    if settings.ai.auto_link_notes {
        if let Some(provider) = settings
            .ai
            .providers
            .iter()
            .find(|p| p.id == settings.ai.active_provider_id)
            .cloned()
        {
            let vault_path = vault.path.clone();
            let note_path = created.clone();
            tauri::async_runtime::spawn(async move {
                match suggest_links_for_note(&vault_path, &note_path, &provider).await {
                    Ok(targets) if !targets.is_empty() => {
                        let ops: Vec<links::LinkOperation> = targets
                            .into_iter()
                            .map(|target| links::LinkOperation {
                                source: note_path.clone(),
                                target,
                                action: links::LinkAction::add,
                            })
                            .collect();
                        if let Err(e) =
                            links::apply_operations(&vault_path, &ops, links::LinkOrigin::agent)
                        {
                            eprintln!("auto-link apply failed for {note_path}: {e}");
                        }
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("auto-link suggestion failed for {note_path}: {e}"),
                }
            });
        }
    }

    if let Some(net) = state.network.read().as_ref() {
        net.sync_now();
    }
    Ok(created)
}

#[tauri::command]
pub fn create_folder(path: String, state: State<'_, AppState>) -> Result<(), String> {
    let settings = state.settings.read();
    let vault = settings.active_vault().ok_or("errors.noActiveVault")?;
    vault::create_folder(&vault.path, &path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn rename_item(
    old_path: String,
    new_path: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let settings = state.settings.read();
    let vault = settings.active_vault().ok_or("errors.noActiveVault")?;
    vault::rename_item(&vault.path, &old_path, &new_path).map_err(|e| e.to_string())?;
    state.crdt.remove_doc(&vault.path, &old_path).map_err(|e| e.to_string())?;
    if let Some(net) = state.network.read().as_ref() {
        net.broadcast_delete(old_path);
        net.sync_now();
    }
    Ok(())
}

#[tauri::command]
pub fn delete_item(path: String, state: State<'_, AppState>) -> Result<(), String> {
    let settings = state.settings.read();
    let vault = settings.active_vault().ok_or("errors.noActiveVault")?;
    // The undo history lives only in AppState: hiding to tray retains it, exiting clears it.
    let mut undo = state.undo.lock();
    let snapshot = DeletedSnapshot::capture(&vault.path, &path).map_err(|e| e.to_string())?;
    vault::delete_item(&vault.path, &path).map_err(|e| e.to_string())?;
    if let Err(error) = state.crdt.remove_doc(&vault.path, &path) {
        eprintln!("Failed to remove CRDT state for {path}: {error}");
    }
    undo.push(vault.id.clone(), snapshot);

    if let Some(net) = state.network.read().as_ref() {
        net.broadcast_delete(path);
    }

    Ok(())
}

#[tauri::command]
pub fn undo_last_delete(state: State<'_, AppState>) -> Result<Option<RestoredItem>, String> {
    let settings = state.settings.read();
    let vault = settings.active_vault().ok_or("errors.noActiveVault")?;
    let restored = state.undo.lock().undo_last_for(&vault.id, &vault.path)
        .map_err(|e| e.to_string())?;
    if restored.is_some() {
        if let Some(net) = state.network.read().as_ref() {
            net.sync_now();
        }
    }
    Ok(restored)
}

#[tauri::command]
pub fn crdt_apply_client_update(
    note_path: String,
    update_base64: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let update_bytes = URL_SAFE_NO_PAD
        .decode(&update_base64)
        .map_err(|_e| "errors.invalidBase64".to_string())?;

    let settings = state.settings.read();
    let vault = settings.active_vault().ok_or("errors.noActiveVault")?;
    let result = state.crdt.apply_update(&vault.path, &note_path, &update_bytes)
        .map_err(|e| e.to_string())?;

    // Full state lets a peer recover from a missed incremental packet.
    if result.changed {
        if let Some(net) = state.network.read().as_ref() {
            net.broadcast_crdt_update(note_path, result.state);
        }
    }

    Ok(())
}

#[tauri::command]
pub fn network_sync_now(state: State<'_, AppState>) -> Result<(), String> {
    let net = state.network.read();
    let service = net.as_ref().ok_or("errors.p2pNotStarted")?;
    service.sync_now();
    Ok(())
}

#[tauri::command]
pub fn network_request_pair(pair_code: String, state: State<'_, AppState>) -> Result<(), String> {
    let invite = decode_pair_code(&pair_code).map_err(|e| e.to_string())?;
    let net = state.network.read();
    let service = net.as_ref().ok_or("errors.p2pNotStarted")?;
    service.request_pair(invite);
    Ok(())
}

#[tauri::command]
pub fn network_answer_pair(
    request_id: String,
    accept: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let net = state.network.read();
    let service = net.as_ref().ok_or("errors.p2pNotStarted")?;
    service.answer_pair(request_id, accept);
    Ok(())
}

#[tauri::command]
pub fn network_remove_peer(
    endpoint_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut settings = state.settings.write();
    if let Some(vault) = settings.active_vault_mut() {
        let removed = vault.remove_peer(&endpoint_id);
        if removed {
            let updated_peers = vault.peers.clone();
            let _ = settings.save();
            drop(settings);

            if let Some(net) = state.network.read().as_ref() {
                net.update_peers(updated_peers);
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn network_broadcast_awareness(
    note_path: String,
    update: Vec<u8>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let net = state.network.read();
    let service = net.as_ref().ok_or("errors.p2pNotStarted")?;
    service.broadcast_awareness(note_path, update);
    Ok(())
}

#[tauri::command]
pub async fn network_get_pair_info(state: State<'_, AppState>) -> Result<Option<PairInfo>, String> {
    for _ in 0..40 {
        let (has_service, maybe_info) = {
            let net = state.network.read();
            match net.as_ref() {
                Some(service) => (true, service.pair_info()),
                None => (false, None),
            }
        };

        if let Some(info) = maybe_info {
            return Ok(Some(info));
        }
        if !has_service {
            return Ok(None);
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let maybe_info = {
        state.network.read().as_ref().and_then(|s| s.pair_info())
    };
    Ok(maybe_info)
}

#[tauri::command]
pub fn save_ai_settings(settings: AiSettings, state: State<'_, AppState>) -> Result<(), String> {
    let mut s = state.settings.write();
    s.ai = settings;
    s.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_web_search_settings(mut settings: WebSearchSettings, state: State<'_, AppState>) -> Result<(), String> {
    settings.normalize();
    if ["brave", "parallel"].into_iter().any(|id| settings.source(id).is_some_and(|source| source.enabled && source.api_key.trim().is_empty())) {
        return Err("ai.webKeyRequired".into());
    }
    if settings.source("searxng").is_some_and(|source| source.enabled) {
        let url = reqwest::Url::parse(settings.searxng_url.trim()).map_err(|_| "ai.webInvalidSearxngUrl")?;
        if url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some()
            || url.host_str().is_some_and(|host| host == "localhost" || host.parse::<std::net::IpAddr>().is_ok()) {
            return Err("ai.webInvalidSearxngUrl".into());
        }
    }
    let mut app = state.settings.write();
    let previous = std::mem::replace(&mut app.web_search, settings);
    if let Err(error) = app.save() {
        app.web_search = previous;
        return Err(error.to_string());
    }
    Ok(())
}

#[tauri::command]
pub fn save_theme(theme: String, state: State<'_, AppState>) -> Result<(), String> {
    if !matches!(theme.as_str(), "dark" | "light") {
        return Err("errors.invalidTheme".to_string());
    }
    let mut settings = state.settings.write();
    let previous = std::mem::replace(&mut settings.theme, theme);
    if let Err(error) = settings.save() {
        settings.theme = previous;
        return Err(error.to_string());
    }
    Ok(())
}

#[tauri::command]
pub fn save_theme_palettes(palettes: ThemePalettesSettings, state: State<'_, AppState>) -> Result<(), String> {
    validate_theme_palettes(&palettes)?;
    let mut settings = state.settings.write();
    let previous = std::mem::replace(&mut settings.theme_palettes, palettes);
    if let Err(error) = settings.save() {
        settings.theme_palettes = previous;
        return Err(error.to_string());
    }
    Ok(())
}

fn validate_theme_palettes(palettes: &ThemePalettesSettings) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    for palette in &palettes.custom_palettes {
        let id = palette.id.trim();
        if id.is_empty()
            || palette.name.trim().is_empty()
            || BUILTIN_PALETTE_IDS.contains(&id)
            || !ids.insert(id.to_string())
            || !palette.dark.is_valid()
            || !palette.light.is_valid()
        {
            return Err("errors.invalidPalette".to_string());
        }
    }
    if !BUILTIN_PALETTE_IDS.contains(&palettes.active_palette_id.as_str())
        && !ids.contains(palettes.active_palette_id.as_str())
    {
        return Err("errors.invalidPalette".to_string());
    }
    Ok(())
}

#[tauri::command]
pub fn save_view_mode(view_mode: String, state: State<'_, AppState>) -> Result<(), String> {
    if !matches!(view_mode.as_str(), "edit" | "split" | "preview") {
        return Err("errors.invalidViewMode".to_string());
    }
    let mut settings = state.settings.write();
    let previous = std::mem::replace(&mut settings.view_mode, view_mode);
    if let Err(error) = settings.save() {
        settings.view_mode = previous;
        return Err(error.to_string());
    }
    Ok(())
}

#[tauri::command]
pub fn save_line_wrapping(line_wrapping: bool, state: State<'_, AppState>) -> Result<(), String> {
    let mut settings = state.settings.write();
    let previous = std::mem::replace(&mut settings.line_wrapping, line_wrapping);
    if let Err(error) = settings.save() {
        settings.line_wrapping = previous;
        return Err(error.to_string());
    }
    Ok(())
}

#[tauri::command]
pub fn save_language(language: String, state: State<'_, AppState>) -> Result<(), String> {
    if !matches!(language.as_str(), "en-US" | "pt-BR" | "es-ES") {
        return Err("errors.invalidLanguage".to_string());
    }
    let mut settings = state.settings.write();
    let previous = std::mem::replace(&mut settings.language, language);
    if let Err(error) = settings.save() {
        settings.language = previous;
        return Err(error.to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn fetch_ai_models(
    provider_id: Option<String>,
    custom_url: Option<String>,
    custom_key: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let (base_url, api_key) = if let Some(url) = custom_url {
        (url, custom_key.unwrap_or_default())
    } else {
        let settings = state.settings.read();
        let p_id = provider_id.unwrap_or_else(|| settings.ai.active_provider_id.clone());
        let provider = settings
            .ai
            .providers
            .iter()
            .find(|p| p.id == p_id)
            .ok_or_else(|| "errors.providerNotFound".to_string())?;
        (provider.base_url.clone(), provider.api_key.clone())
    };

    rag::fetch_models(&base_url, &api_key)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn search_vault_rag(
    query: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<RagChunk>, String> {
    let settings = state.settings.read();
    let vault = settings.active_vault().ok_or("errors.noActiveVault")?;
    rag::index_and_search_vault(&vault.path, &query, limit.unwrap_or(5))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn ai_chat_query(
    prompt: String,
    note_path_scope: Option<String>,
    conversation: Vec<ChatMessage>,
    skill: Option<AssistantSkill>,
    state: State<'_, AppState>,
) -> Result<ChatResponse, String> {
    if prompt.trim().is_empty() {
        return Err("ai.emptyPrompt".into());
    }
    let skill = skill.unwrap_or_default();
    let (provider, vault_path, vault_id, web_settings, language) = {
        let settings = state.settings.read();
        let vault = settings.active_vault().ok_or("errors.noActiveVault")?;
        let p = settings
            .ai
            .providers
            .iter()
            .find(|p| p.id == settings.ai.active_provider_id)
            .cloned()
            .ok_or("nenhum provedor de IA selecionado")?;
        (p, vault.path.clone(), vault.id.clone(),
            settings.web_search.clone(), settings.language.clone())
    };

    let web_sources = if skill == AssistantSkill::Research {
        web_search::search_web(&prompt, &web_settings, &language)
            .await.map_err(|e| e.to_string())?
    } else {
        Vec::new()
    };

    let (sources, context_text) = if let Some(path) = &note_path_scope {
        let content = vault::read_note(&vault_path, path).map_err(|e| e.to_string())?;
        let chunk = RagChunk {
            note_path: path.clone(),
            note_title: path.clone(),
            section_title: "Open Note".to_string(),
            line_number: 1,
            content: content.clone(),
            score: 1.0,
        };
        let context = format!("---\n[Open Note: {path} (Line: 1)]\n{content}\n---");
        (vec![chunk], context)
    } else {
        let chunks = rag::index_and_search_vault(&vault_path, &prompt, 5)
            .map_err(|e| e.to_string())?;

        let mut parts = Vec::new();
        for chunk in &chunks {
            parts.push(format!(
                "---\n[Note: {} ({}), Section: {}, Line: {}]\n{}\n---",
                chunk.note_title, chunk.note_path, chunk.section_title, chunk.line_number, chunk.content
            ));
        }
        let context = parts.join("\n\n");
        (chunks, context)
    };

    let saved_memory = chat_history::load(&vault_id)
        .map(|history| history.memory)
        .unwrap_or_default();
    let context = serde_json::json!({
        "notes": context_text.chars().take(48_000).collect::<String>(),
        "saved_user_memory": saved_memory.chars().take(8_000).collect::<String>(),
        "web_search_performed": skill == AssistantSkill::Research,
        "web_results": web_sources,
    }).to_string();
    let messages = assistant::build_messages(skill, &prompt, &context, &conversation);

    let answer = rag::generate_chat_completion(&provider, &messages, 0.3)
        .await
        .map_err(|e| e.to_string())?;

    let (answer, drafts, mut warnings) = if skill == AssistantSkill::Notes {
        (answer, Vec::new(), Vec::new())
    } else {
        assistant::extract_drafts(&answer)
    };
    let (answer, edits, edit_warnings) = if skill == AssistantSkill::Notes {
        (answer, Vec::new(), Vec::new())
    } else {
        assistant::extract_edits(&answer, &sources)
    };
    warnings.extend(edit_warnings);
    if skill == AssistantSkill::Research && web_sources.is_empty() {
        warnings.push("ai.webNoResults".into());
    }
    Ok(ChatResponse { answer, sources, web_sources, drafts, edits, warnings, vault_id })
}

#[tauri::command]
pub fn chat_history_get(vault_id: String, state: State<'_, AppState>) -> Result<ChatHistory, String> {
    if !state.settings.read().vaults.iter().any(|vault| vault.id == vault_id) {
        return Err("ai.vaultChanged".into());
    }
    chat_history::load(&vault_id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn chat_history_save(
    vault_id: String,
    history: ChatHistory,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if !state.settings.read().vaults.iter().any(|vault| vault.id == vault_id) {
        return Err("ai.vaultChanged".into());
    }
    chat_history::save(&vault_id, &history).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn ai_save_draft(
    vault_id: String,
    draft: NoteDraft,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let settings = state.settings.read();
    let vault = settings.active_vault().ok_or("errors.noActiveVault")?;
    if vault.id != vault_id {
        return Err("ai.vaultChanged".into());
    }
    let path = assistant::save_draft(&vault.path, &draft).map_err(|e| e.to_string())?;
    if let Err(e) = links::reconcile_wikilinks(&vault.path, &path, &draft.content) {
        eprintln!("reconcile_wikilinks failed for {path}: {e}");
    }
    if let Some(net) = state.network.read().as_ref() {
        net.sync_now();
    }
    Ok(path)
}

#[tauri::command]
pub fn ai_apply_edit(vault_id: String, edit: NoteEdit, state: State<'_, AppState>, app: AppHandle) -> Result<String, String> {
    let settings = state.settings.read();
    let vault = settings.active_vault().ok_or("errors.noActiveVault")?;
    if vault.id != vault_id { return Err("ai.vaultChanged".into()); }
    let result = state.crdt.apply_note_edit(&vault.path, &edit).map_err(|e| e.to_string())?;
    // The open editor receives the same CRDT state as the paired devices.
    let _ = app.emit("p2p:crdt-update", crate::network::NetworkEventPayload::RemoteCrdtUpdate {
        note_path: edit.path.clone(), update: result.state.clone(),
    });
    if let Some(net) = state.network.read().as_ref() {
        net.broadcast_crdt_update(edit.path.clone(), result.state);
        net.sync_now();
    }
    Ok(edit.path)
}

/// Load an image from the vault or its explicit HTTP(S) source for export.
#[tauri::command]
pub async fn load_export_image(note_path: String, src: String, state: State<'_, AppState>) -> Result<crate::export_images::ExportImageData, String> {
    let root = state.settings.read().active_vault().ok_or("errors.noActiveVault")?.path.clone();
    crate::export_images::load_image(&root, &note_path, &src).await.map_err(|_| "export.imageFailed".into())
}

/// Destination is chosen using the native save dialog, not generated by the model.
#[tauri::command]
pub fn export_document(path: String, bytes: Vec<u8>) -> Result<(), String> {
    let path = Path::new(&path);
    let extension = path.extension().and_then(|s| s.to_str())
        .unwrap_or_default().to_ascii_lowercase();
    let valid = match extension.as_str() {
        "pdf" => bytes.starts_with(b"%PDF-"),
        "docx" => bytes.starts_with(b"PK\x03\x04"),
        _ => false,
    };
    if !valid || bytes.len() > 32 * 1024 * 1024 {
        return Err("export.invalidFile".into());
    }
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn mark_welcome_seen(state: State<'_, AppState>) -> Result<(), String> {
    let mut s = state.settings.write();
    s.has_seen_welcome = true;
    s.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn links_get(state: State<'_, AppState>) -> Result<Vec<links::LinkEdge>, String> {
    let settings = state.settings.read();
    let vault = settings.active_vault().ok_or("errors.noActiveVault")?;
    links::graph_links(&vault.path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn links_apply(
    operations: Vec<links::LinkOperation>,
    origin: Option<links::LinkOrigin>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let settings = state.settings.read();
    let vault = settings.active_vault().ok_or("errors.noActiveVault")?;
    links::apply_operations(
        &vault.path,
        &operations,
        origin.unwrap_or(links::LinkOrigin::manual),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn ai_suggest_links(
    note_path: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let (provider, vault_path) = {
        let settings = state.settings.read();
        let vault = settings.active_vault().ok_or("errors.noActiveVault")?;
        let provider = settings
            .ai
            .providers
            .iter()
            .find(|p| p.id == settings.ai.active_provider_id)
            .cloned()
            .ok_or("errors.providerNotFound")?;
        (provider, vault.path.clone())
    };

    suggest_links_for_note(&vault_path, &note_path, &provider)
        .await
        .map_err(|e| {
            eprintln!("ai_suggest_links failed for {note_path}: {e}");
            "errors.suggestLinksFailed".to_string()
        })
}

/// Ask the active AI provider for up to 5 vault notes related to `note_path`.
/// Shared by `ai_suggest_links` and the create-note auto-link background task.
async fn suggest_links_for_note(
    vault_path: &Path,
    note_path: &str,
    provider: &AiProviderConfig,
) -> anyhow::Result<Vec<String>> {
    let content = vault::read_note(vault_path, note_path)?;
    let items = vault::list_vault_items(vault_path)?;

    let others: Vec<String> = items
        .iter()
        .filter(|i| !i.is_dir && i.path != note_path)
        .map(|i| format!("{} | {}", i.path, i.title))
        .collect();
    if others.is_empty() {
        return Ok(Vec::new());
    }

    let truncated: String = content.chars().take(4000).collect();
    let user_prompt = format!(
        "Note path: {note_path}\n\nNote content:\n{truncated}\n\nOther notes (path | title):\n{}",
        others.join("\n")
    );
    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: "You are a note-linking assistant. Respond ONLY with a JSON array of up to 5 relative note paths most related to the given note. Exclude the note itself.".to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_prompt,
        },
    ];

    let answer = rag::generate_chat_completion(provider, &messages, 0.2).await?;

    let mut out: Vec<String> = Vec::new();
    for candidate in parse_ai_path_list(&answer) {
        let clean = candidate.trim().replace('\\', "/");
        if clean.is_empty() || clean == note_path {
            continue;
        }
        let full = match vault::safe_join(vault_path, &clean) {
            Ok(f) => f,
            Err(_) => continue,
        };
        if !full.is_file() || !vault::is_markdown(&full) {
            continue;
        }
        if !out.contains(&clean) {
            out.push(clean);
        }
        if out.len() >= 5 {
            break;
        }
    }
    Ok(out)
}

/// Defensive parse of an AI answer expected to be a JSON array of strings.
/// Strips code fences; falls back to extracting double-quoted substrings.
fn parse_ai_path_list(answer: &str) -> Vec<String> {
    let trimmed = answer.trim();
    let without_open = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .trim();
    let stripped = without_open.strip_suffix("```").unwrap_or(without_open).trim();

    if let Ok(parsed) = serde_json::from_str::<Vec<String>>(stripped) {
        return parsed;
    }

    let mut out = Vec::new();
    let bytes = stripped.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'"' {
                j += 1;
            }
            if j >= bytes.len() {
                break;
            }
            out.push(stripped[start..j].to_string());
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

#[tauri::command]
pub fn save_update_prefs(
    update_check: bool,
    skipped_version: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut settings = state.settings.write();
    settings.update_check = update_check;
    settings.skipped_version = skipped_version;
    settings.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_close_to_tray(close_to_tray: bool, state: State<'_, AppState>) -> Result<(), String> {
    let mut settings = state.settings.write();
    let previous = std::mem::replace(&mut settings.close_to_tray, close_to_tray);
    if let Err(error) = settings.save() {
        settings.close_to_tray = previous;
        return Err(error.to_string());
    }
    Ok(())
}

fn restart_network_service(
    state: &AppState,
    vault: &VaultConfig,
    app: AppHandle,
) -> Result<(), String> {
    let mut vault_clone = vault.clone();
    if vault_clone.ensure_keys() {
        let mut s = state.settings.write();
        if let Some(v) = s.vaults.iter_mut().find(|v| v.id == vault_clone.id) {
            v.secret_key = vault_clone.secret_key.clone();
            v.pairing_token = vault_clone.pairing_token.clone();
            let _ = s.save();
        }
    }
    let secret = vault_clone.secret_key().map_err(|e| e.to_string())?;
    let identity = NetworkIdentity {
        device_name: state.settings.read().device_name.clone(),
        secret_key: secret,
        pairing_token: vault_clone.pairing_token.clone(),
        vault_id: vault_clone.id.clone(),
        vault_name: vault_clone.name.clone(),
    };

    let service = NetworkService::start(
        vault_clone.path.clone(),
        identity,
        vault_clone.peers.clone(),
        app,
        state.settings.clone(),
    );
    *state.network.write() = Some(service);
    Ok(())
}
