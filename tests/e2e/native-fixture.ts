import { expect, type Page } from '@playwright/test';
import * as Y from 'yjs';

/** Exercise the real frontend and IPC arguments against an isolated native adapter.
 * Native persistence/protocol behavior is covered by the Rust integration suite.
 */
export async function openVault(page: Page, options: {
  source?: string; theme?: 'light' | 'dark'; palette?: string;
  credentialError?: string;
  notices?: Array<{ path: string; recovered: boolean }>;
} = {}) {
  const source = options.source ?? '# Search\n\n[Title](https://visible.example)\n\n**hello** world\n\n- [ ] needle task\n\n'
    + Array.from({ length: 12 }, (_, i) => `## Section ${i + 1}\n\n${'Long filler text for scrolling. '.repeat(8)}\n\nneedle checkpoint ${i + 1}\n\n`).join('') + 'needles plural';
  const doc = new Y.Doc();
  const text = doc.getText('content');
  text.insert(0, source);
  const errors: string[] = [];
  const openedUrls: string[] = [];
  const saves: unknown[] = [];
  let notices = options.notices ?? [];
  let history = { version: 1, activeConversationId: null, conversations: [], memory: '' };
  const vault = { id: 'fixture', name: 'Test vault', path: 'fixture', peers: [] };
  const item = { path: 'Lista.md', name: 'Lista', title: 'Lista', is_dir: false, size: source.length, modified_ms: 1 };
  const settings = {
    credential_error: options.credentialError ?? '',
    device_name: 'Test device', theme: options.theme ?? 'light',
    theme_palettes: { active_palette_id: options.palette ?? 'lowbloat', custom_palettes: [] },
    view_mode: 'split', line_wrapping: true, language: 'en-US', update_check: false, close_to_tray: true,
    skipped_version: '', active_vault_id: vault.id, vaults: [vault], has_seen_welcome: true,
    ai: { active_provider_id: 'ollama', providers: [{ id: 'ollama', name: 'Ollama', base_url: 'http://localhost:11434/v1', selected_model: 'fixture', api_key: '', is_custom: false }], auto_link_notes: false },
    web_search: { sources: [] }, image_upload: { provider: 'local', imgur_client_id: '', local_default_applied: true },
  };
  page.on('pageerror', error => errors.push(error.message));
  await page.exposeFunction('nativeInvokeMock', (command: string, args: Record<string, any>) => {
    switch (command) {
      case 'get_app_state': return { settings, active_vault: vault, items: [item], pair_info: null };
      case 'list_notes': return [item];
      case 'read_note': return { content: text.toString(), crdt_update_base64: Buffer.from(Y.encodeStateAsUpdate(doc)).toString('base64url') };
      case 'crdt_apply_client_update': saves.push(args); Y.applyUpdate(doc, Buffer.from(args.updateBase64, 'base64url')); return null;
      case 'save_view_mode': settings.view_mode = args.viewMode; return null;
      case 'save_theme': settings.theme = args.theme; return null;
      case 'chat_history_get': return history;
      case 'chat_history_save': history = args.history; return null;
      case 'take_recovery_notices': { const result = notices; notices = []; return result; }
      case 'links_get': return [];
      case 'retry_credentials': settings.credential_error = ''; return settings;
      case 'get_update_policy': return { channel: 'windows', can_install: true, updater_target: null };
      case 'plugin:app|version': return '0.3.3';
      case 'plugin:opener|open_url': openedUrls.push(args.url); return null;
      default: return null;
    }
  });
  await page.addInitScript(() => {
    let id = 0;
    const callbacks: Record<number, (event: unknown) => void> = {};
    const listeners: Record<string, number[]> = {};
    const globals = window as typeof window & Record<string, any>;
    globals.__callbacks = callbacks;
    globals.__listeners = listeners;
    globals.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener(event: string, eventId: number) {
      listeners[event] = (listeners[event] ?? []).filter(value => value !== eventId);
    } };
    globals.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
      transformCallback(fn: (event: unknown) => void) { callbacks[++id] = fn; return id; },
      unregisterCallback(callbackId: number) { delete callbacks[callbackId]; },
      convertFileSrc(path: string, protocol = 'asset') { return `http://${protocol}.localhost/${encodeURIComponent(path)}`; },
      async invoke(command: string, args: Record<string, any> = {}) {
        if (command === 'plugin:event|listen') { (listeners[args.event] ??= []).push(args.handler); return ++id; }
        return globals.nativeInvokeMock(command, args);
      },
    };
  });
  await page.goto('/');
  await expect(page.locator('.cm-content')).toBeVisible();
  return {
    source, errors, openedUrls, saves,
    content: () => text.toString(),
    async remoteAppend(content: string) {
      text.insert(text.length, content);
      const update = Array.from(Y.encodeStateAsUpdate(doc));
      await page.evaluate(update => {
        const globals = window as typeof window & Record<string, any>;
        for (const handler of globals.__listeners['p2p:crdt-update'] ?? []) {
          globals.__callbacks[handler]({ event: 'p2p:crdt-update', id: handler,
            payload: { type: 'RemoteCrdtUpdate', note_path: 'Lista.md', update } });
        }
      }, update);
    },
    destroy() { doc.destroy(); },
  };
}
