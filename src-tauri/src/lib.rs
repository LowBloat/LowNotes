pub mod config;
pub mod crdt;
pub mod network;
pub mod vault;
pub mod commands;
pub mod rag;
pub mod links;
pub mod link_operations;
pub mod catalog;
pub mod structural;
pub mod catalog_sync;
pub mod references;
pub mod reference_sync;
pub mod assistant;
pub mod web_search;
pub mod chat_history;

pub mod export_images;
pub mod export_target;
pub mod http;
pub mod logging;
pub mod updates;
pub mod image_upload;
pub mod local_images;
pub mod storage;
pub mod note_transaction;
pub mod creation;
pub mod note_history;
pub mod retention;
pub mod history_commands;
pub mod credentials;

use std::sync::Arc;
use parking_lot::RwLock;
use tauri::{Manager, menu::{Menu, MenuItem}, tray::{MouseButton, TrayIconBuilder, TrayIconEvent}};

use config::AppSettings;
use crdt::CrdtManager;
use network::{NetworkIdentity, NetworkService};
use commands::AppState;


/// Tray menu labels for the interface language. An empty or unknown code
/// (automatic detection happens in the interface) falls back to English until
/// the interface reports its resolved language.
pub fn tray_labels(language: &str) -> (&'static str, &'static str) {
    match language {
        "pt-BR" => ("Abrir LowNotes", "Sair do LowNotes"),
        "es-ES" => ("Abrir LowNotes", "Salir de LowNotes"),
        _ => ("Open LowNotes", "Quit LowNotes"),
    }
}

struct TrayMenu {
    open: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}

/// Called by the interface whenever its resolved language changes.
#[tauri::command]
fn set_tray_language(language: String, app: tauri::AppHandle) -> Result<(), String> {
    if let Some(menu) = app.try_state::<TrayMenu>() {
        let (open, quit) = tray_labels(&language);
        menu.open.set_text(open).map_err(|e| e.to_string())?;
        menu.quit.set_text(quit).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn show_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    logging::init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "LowNotes starting");
    let settings = Arc::new(RwLock::new(AppSettings::load()));
    let crdt = CrdtManager::new();
    let network = Arc::new(RwLock::new(None::<NetworkService>));

    let app_state = AppState {
        settings: settings.clone(),
        crdt,
        network: network.clone(),

    };

    let builder = tauri::Builder::default();

    // Register first so duplicate launches exit before the tray and network start.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        show_main_window(app);
    }));

    let update_policy = updates::UpdatePolicy::detect();
    let builder = if update_policy.can_install {
        let mut updater = tauri_plugin_updater::Builder::new();
        if let Some(target) = &update_policy.updater_target {
            updater = updater.target(target);
        }
        builder.plugin(updater.build())
    } else {
        // Native packages and read-only AppImages cannot invoke updater installation.
        builder
    };

    builder
        .register_asynchronous_uri_scheme_protocol("lownotes-image", |context, request, responder| {
            let app = context.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                let state = app.state::<AppState>();
                let settings = state.settings.read().clone();
                responder.respond(local_images::protocol_response(&settings, request.uri().path()));
            });
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .manage(update_policy)
        .manage(app_state)
        .setup(move |app| {
            let projection_app = app.handle().clone();
            app.state::<AppState>().crdt.set_projection_observer(Arc::new(move |_root, path, state| {
                use tauri::Emitter;
                let _ = projection_app.emit("p2p:crdt-update", network::NetworkEventPayload::RemoteCrdtUpdate {
                    note_path: path.into(), update: state.into(),
                });
            }));
            let mut s = settings.write();
            let mut saved_keys = false;
            if let Some(vault) = s.active_vault_mut() {
                saved_keys = vault.ensure_keys();
            }
            // A freshly generated identity that cannot be saved would be replaced
            // by another one on the next start, breaking every pairing. Do not
            // go online with it; Settings shows the credential error and Retry.
            let mut identity_unsaved = false;
            if saved_keys {
                if let Err(error) = s.save() {
                    tracing::error!("could not save the vault identity: {error:#}");
                    s.credential_error = "credentials.unavailable".into();
                    identity_unsaved = true;
                }
            }
            if let Some(vault) = s.active_vault().filter(|_| !identity_unsaved) {
                if let Ok(secret) = vault.secret_key() {
                    let identity = NetworkIdentity {
                        device_name: s.device_name.clone(),
                        secret_key: secret,
                        pairing_token: vault.pairing_token.clone(),
                        vault_id: vault.id.clone(),
                        vault_name: vault.name.clone(),
                    };
                    let service = NetworkService::start(
                        vault.path.clone(),
                        identity,
                        vault.peers.clone(),
                        app.handle().clone(),
                        settings.clone(),
                    );
                    *network.write() = Some(service);
                }
            }
            drop(s);
            history_commands::start_retention(settings.clone(), app.state::<AppState>().crdt.clone());

            let (open_label, quit_label) = tray_labels(&settings.read().language);
            let open = MenuItem::with_id(app, "open", open_label, true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", quit_label, true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            app.manage(TrayMenu { open: open.clone(), quit: quit.clone() });
            TrayIconBuilder::new()
                .icon(app.default_window_icon().expect("ícone do aplicativo").clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, .. } = event {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    if window.app_handle().state::<AppState>().settings.read().close_to_tray {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            set_tray_language,
            commands::take_recovery_notices,
            commands::retry_credentials,
            history_commands::history_list,
            history_commands::history_version,
            history_commands::history_apply,
            history_commands::history_trash_read,
            history_commands::history_trash_restore,
            history_commands::history_retention_save,
            history_commands::history_cleanup,
            image_upload::upload_clipboard_image,
            image_upload::save_image_upload_settings,
            updates::get_update_policy,
            updates::check_external_update,
            commands::get_app_state,
            commands::chat_history_get,
            commands::chat_history_save,
            commands::select_vault,
            commands::create_vault,
            commands::forget_vault,
            commands::list_notes,
            commands::read_note,
            commands::save_note,
            commands::create_note,
            commands::create_folder,
            commands::rename_item,
            commands::delete_item,
            commands::undo_last_delete,
            commands::crdt_apply_client_update,
            commands::network_sync_now,
            commands::network_request_pair,
            commands::network_answer_pair,
            commands::network_regenerate_pair_code,
            commands::network_remove_peer,
            commands::network_broadcast_awareness,
            commands::save_ai_settings,
            commands::save_web_search_settings,
            commands::network_get_pair_info,
            commands::save_theme,
            commands::save_theme_palettes,
            commands::save_view_mode,
            commands::save_line_wrapping,
            commands::save_remote_images,
            commands::save_language,
            commands::fetch_ai_models,
            commands::search_vault_rag,
            commands::ai_chat_query,
            commands::ai_save_draft,
            commands::ai_apply_edit,
            commands::export_choose_destination,
            commands::export_document,
            commands::load_export_image,
            commands::mark_welcome_seen,
            commands::links_get,
            commands::links_apply,
            commands::ai_suggest_links,
            commands::save_update_prefs,
            commands::save_close_to_tray,
            commands::open_log_folder,
        ])
        .run(tauri::generate_context!())
        .expect("erro ao executar o aplicativo LowNotes");
}

#[cfg(test)]
mod tests {
    use super::tray_labels;

    #[test]
    fn tray_labels_follow_the_interface_language() {
        assert_eq!(tray_labels("pt-BR"), ("Abrir LowNotes", "Sair do LowNotes"));
        assert_eq!(tray_labels("es-ES"), ("Abrir LowNotes", "Salir de LowNotes"));
        assert_eq!(tray_labels("en-US"), ("Open LowNotes", "Quit LowNotes"));
        assert_eq!(tray_labels(""), ("Open LowNotes", "Quit LowNotes"));
    }
}
