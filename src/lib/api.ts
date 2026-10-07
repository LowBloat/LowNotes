import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import type {
  AiSettings,
  AppSettings,
  AssistantSkill,
  NoteDraft,
  AppTheme,
  ChatMessage,
  ChatHistory,
  ChatResponse,
  InitialStateResponse,
  UpdatePolicy,
  ReleaseNotice,
  LinkEdge,
  LinkOperation,
  LinkOrigin,
  NoteReadResponse,
  PairInfo,
  RagChunk,
  VaultItem,
  ViewMode,
  ThemePalettesSettings,
  WebSearchSettings,
  ImageUploadProvider,
  ImageUploadSettings,
  NoteEdit,
} from './types';

export async function uploadClipboardImage(bytes: Uint8Array, provider: ImageUploadProvider, vaultId = ''): Promise<string> {
  return await invoke('upload_clipboard_image', bytes, { headers: { 'x-upload-provider': provider, 'x-vault-id': vaultId } });
}

export function localImageUrl(src: string, vaultId: string, revision = 0): string {
  if (!/^lownotes-image:[0-9a-f]{64}\.(png|jpg|gif|webp|bmp)$/.test(src) || !vaultId) return src;
  return convertFileSrc(`${vaultId}/${src.slice('lownotes-image:'.length)}`, 'lownotes-image') + `?revision=${revision}`;
}

export async function saveImageUploadSettings(settings: ImageUploadSettings): Promise<void> {
  await invoke('save_image_upload_settings', { settings });
}

export async function getAppState(): Promise<InitialStateResponse> {
  return await invoke('get_app_state');
}

export async function takeRecoveryNotices(): Promise<Array<{ path: string; recovered: boolean }>> {
  return await invoke('take_recovery_notices');
}

export async function retryCredentials(): Promise<AppSettings> {
  return await invoke('retry_credentials');
}

export async function getUpdatePolicy(): Promise<UpdatePolicy> {
  return await invoke('get_update_policy');
}

export async function checkExternalUpdate(): Promise<ReleaseNotice | null> {
  return await invoke('check_external_update');
}

export async function selectVault(path: string): Promise<InitialStateResponse> {
  return await invoke('select_vault', { pathStr: path });
}

export async function createVault(path: string, name?: string): Promise<InitialStateResponse> {
  return await invoke('create_vault', { pathStr: path, name });
}

export async function listNotes(): Promise<VaultItem[]> {
  return await invoke('list_notes');
}

export async function readNote(path: string): Promise<NoteReadResponse> {
  return await invoke('read_note', { path });
}

export async function saveNote(path: string, content: string): Promise<void> {
  return await invoke('save_note', { path, content });
}

export async function createNote(path: string, title?: string): Promise<string> {
  return await invoke('create_note', { path, title });
}

export async function createFolder(path: string): Promise<void> {
  return await invoke('create_folder', { path });
}

export async function renameItem(oldPath: string, newPath: string): Promise<void> {
  return await invoke('rename_item', { oldPath, newPath });
}

export async function deleteItem(path: string): Promise<void> {
  return await invoke('delete_item', { path });
}

export async function undoLastDelete(): Promise<{ path: string; is_dir: boolean; has_more: boolean } | null> {
  return await invoke('undo_last_delete');
}

export async function crdtApplyClientUpdate(notePath: string, updateBase64: string): Promise<void> {
  return await invoke('crdt_apply_client_update', { notePath, updateBase64 });
}

export async function networkSyncNow(): Promise<void> {
  return await invoke('network_sync_now');
}

export async function networkRequestPair(pairCode: string): Promise<void> {
  return await invoke('network_request_pair', { pairCode });
}

export async function networkAnswerPair(requestId: string, accept: boolean): Promise<void> {
  return await invoke('network_answer_pair', { requestId, accept });
}

export async function networkRemovePeer(endpointId: string): Promise<void> {
  return await invoke('network_remove_peer', { endpointId });
}

export async function broadcastAwareness(notePath: string, update: number[]): Promise<void> {
  return await invoke('network_broadcast_awareness', { notePath, update });
}

export async function networkGetPairInfo(): Promise<PairInfo | null> {
  return await invoke('network_get_pair_info');
}

export async function saveLanguage(language: string): Promise<void> {
  return await invoke('save_language', { language });
}

export async function linksGet(): Promise<LinkEdge[]> {
  return await invoke('links_get');
}

export async function linksApply(operations: LinkOperation[], origin?: LinkOrigin): Promise<void> {
  return await invoke('links_apply', { operations, origin });
}

export async function aiSuggestLinks(notePath: string): Promise<string[]> {
  return await invoke('ai_suggest_links', { notePath });
}

export async function saveUpdatePrefs(updateCheck: boolean, skippedVersion: string): Promise<void> {
  return await invoke('save_update_prefs', { updateCheck, skippedVersion });
}

export async function saveCloseToTray(closeToTray: boolean): Promise<void> {
  return await invoke('save_close_to_tray', { closeToTray });
}

export async function pickVaultDirectory(): Promise<string | null> {
  const selected = await open({
    directory: true,
    multiple: false,
    title: 'Selecione a pasta para o Vault',
  });
  if (typeof selected === 'string') {
    return selected;
  }
  return null;
}

export async function saveAiSettings(settings: AiSettings): Promise<void> {
  return await invoke('save_ai_settings', { settings });
}

export async function saveWebSearchSettings(settings: WebSearchSettings): Promise<void> {
  return await invoke('save_web_search_settings', { settings });
}

export async function saveTheme(theme: AppTheme): Promise<void> {
  return await invoke('save_theme', { theme });
}

export async function saveThemePalettes(palettes: ThemePalettesSettings): Promise<void> {
  return await invoke('save_theme_palettes', { palettes });
}

export async function saveViewMode(viewMode: ViewMode): Promise<void> {
  return await invoke('save_view_mode', { viewMode });
}

export async function saveLineWrapping(lineWrapping: boolean): Promise<void> {
  return await invoke('save_line_wrapping', { lineWrapping });
}

export async function fetchAiModels(
  providerId?: string,
  customUrl?: string,
  customKey?: string
): Promise<string[]> {
  return await invoke('fetch_ai_models', { providerId, customUrl, customKey });
}

export async function searchVaultRag(query: string, limit?: number): Promise<RagChunk[]> {
  return await invoke('search_vault_rag', { query, limit });
}

export async function aiChatQuery(
  prompt: string,
  notePathScope?: string,
  conversation: ChatMessage[] = [],
  skill: AssistantSkill = 'auto',
): Promise<ChatResponse> {
  return await invoke('ai_chat_query', { prompt, notePathScope, conversation, skill });
}

export async function aiSaveDraft(vaultId: string, draft: NoteDraft): Promise<string> {
  return await invoke('ai_save_draft', { vaultId, draft });
}

export async function aiApplyEdit(vaultId: string, edit: NoteEdit): Promise<string> {
  return await invoke('ai_apply_edit', { vaultId, edit });
}

export async function chatHistoryGet(vaultId: string): Promise<ChatHistory> {
  return await invoke('chat_history_get', { vaultId });
}

export async function chatHistorySave(vaultId: string, history: ChatHistory): Promise<void> {
  return await invoke('chat_history_save', { vaultId, history });
}

export async function markWelcomeSeen(): Promise<void> {
  return await invoke('mark_welcome_seen');
}
