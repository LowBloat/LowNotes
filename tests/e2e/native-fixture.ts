import { expect, type Page } from '@playwright/test';
import * as Y from 'yjs';
import { createHash } from 'node:crypto';
import type { VaultItem } from '../../src/lib/types';

/** Exercise the real frontend and IPC arguments against an isolated native adapter.
 * Native persistence/protocol behavior is covered by the Rust integration suite.
 */
export async function openVault(page: Page, options: {
  source?: string; theme?: 'light' | 'dark'; palette?: string;
  credentialError?: string;
  notices?: Array<{ path: string; recovered: boolean }>;
  notePath?: string; otherNotes?: VaultItem[];
  history?: { versionContent?: string; comparison?: { path: string; content: string }; trashContent?: string; folderTrash?: boolean };
} = {}) {
  const source = options.source ?? '# Search\n\n[Title](https://visible.example)\n\n**hello** world\n\n- [ ] needle task\n\n'
    + Array.from({ length: 12 }, (_, i) => `## Section ${i + 1}\n\n${'Long filler text for scrolling. '.repeat(8)}\n\nneedle checkpoint ${i + 1}\n\n`).join('') + 'needles plural';
  const doc = new Y.Doc();
  const text = doc.getText('content');
  text.insert(0, source);
  const errors: string[] = [];
  const openedUrls: string[] = [];
  const saves: Array<Record<string, any>> = [];
  const reads: string[] = [];
  const historyActions: Array<{ command: string; args: Record<string, any> }> = [];
  let deleted = false;
  const recovered: string[] = [];
  let notices = options.notices ?? [];
  let history = { version: 1, activeConversationId: null, conversations: [], memory: '' };
  const vault = { id: 'fixture', name: 'Test vault', path: 'fixture', peers: [] };
  const notePath = options.notePath ?? 'Lista.md';
  const item = { path: notePath, name: 'Lista', title: 'Lista', is_dir: false, size: source.length, modified_ms: 1 };
  const items = [item, ...(options.otherNotes ?? [])];
  const documents = new Map<string, Y.Doc>([[notePath, doc]]);
  if (options.history?.comparison) {
    const comparison = options.history.comparison;
    const extra = new Y.Doc(); extra.getText('content').insert(0, comparison.content); documents.set(comparison.path, extra);
    items.push({ path: comparison.path, name: comparison.path, title: comparison.path, is_dir: false, size: comparison.content.length, modified_ms: 1 });
  }
  const version = { id: '1'.repeat(64), note_id: 'fixture-note-id', path: notePath, created_ms: 1791396352000,
    hash: 'a'.repeat(64), characters: options.history?.versionContent?.length ?? 0 };
  let trash = options.history?.trashContent !== undefined ? [{ record_id: '2'.repeat(64), note_id: '3'.repeat(64),
    path: options.history.folderTrash ? 'removed-folder' : 'removed.md', is_dir: options.history.folderTrash ?? false, deleted_ms: 1791396352000, items: options.history.folderTrash ? 3 : 1 }] : [];
  let retention = { versions_days: null as number | null, trash_days: null as number | null };
  const fingerprint = () => createHash('sha256').update(text.toString()).digest('hex');
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
  await page.exposeFunction('nativeInvokeMock', async (command: string, args: Record<string, any>) => {
    if (command.startsWith('history_')) historyActions.push({ command, args });
    switch (command) {
      case 'get_app_state': return { settings, active_vault: vault, items, pair_info: null };
      case 'list_notes': return items;
      case 'read_note': {
        reads.push(args.path); const selected = documents.get(args.path) ?? doc;
        return { content: selected.getText('content').toString(), crdt_update_base64: Buffer.from(Y.encodeStateAsUpdate(selected)).toString('base64url'), note_id: 'fixture-note-id' };
      }
      case 'history_list': return {
        note: args.path === notePath ? { note_id: 'fixture-note-id', path: notePath, content: text.toString(), hash: fingerprint() } : null,
        versions: args.path === notePath && options.history?.versionContent !== undefined ? [version] : [], trash, retention,
      };
      case 'history_version': return { summary: version, content: options.history?.versionContent ?? '' };
      case 'history_apply': {
        if (args.expectedHash !== fingerprint()) throw 'history.noteChanged';
        const replacement = args.versionId ? options.history?.versionContent ?? '' : args.content;
        doc.transact(() => { text.delete(0, text.length); text.insert(0, replacement); });
        await page.evaluate(({ update, path }) => {
          const globals = window as typeof window & Record<string, any>;
          for (const handler of globals.__listeners['p2p:crdt-update'] ?? []) globals.__callbacks[handler]({ payload: { type: 'RemoteCrdtUpdate', note_path: path, update, vault_id: 'fixture' } });
        }, { update: Array.from(Y.encodeStateAsUpdate(doc)), path: notePath });
        return notePath;
      }
      case 'history_trash_read': return options.history?.trashContent ?? '';
      case 'history_trash_restore': {
        const entry = trash[0]; if (!entry) throw 'history.unavailable';
        trash = [];
        const restored = entry.is_dir ? 'restored-folder' : 'removed (restored).md';
        if (!entry.is_dir) {
          const extra = new Y.Doc(); extra.getText('content').insert(0, options.history?.trashContent ?? ''); documents.set(restored, extra);
        }
        items.push({ path: restored, name: restored, title: restored, is_dir: entry.is_dir, size: 1, modified_ms: 1 });
        return { path: restored, is_dir: entry.is_dir };
      }
      case 'history_retention_save': retention = args.policy; return null;
      case 'history_cleanup': return retention.versions_days || retention.trash_days
        ? { versions: 2, archives: 1, protected: 3 } : { versions: 0, archives: 0, protected: 0 };
      case 'crdt_apply_client_update': {
        saves.push(args);
        if (deleted) {
          if (!args.recoveryUpdate) throw 'errors.noteDeleted';
          const copy = new Y.Doc(); Y.applyUpdate(copy, Buffer.from(args.updateBase64, 'base64url'));
          recovered.push(copy.getText('content').toString()); copy.destroy();
        } else Y.applyUpdate(doc, Buffer.from(args.updateBase64, 'base64url'));
        return null;
      }
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
    source, errors, openedUrls, saves, recovered, reads, historyActions,
    deleteWhileEditing() { deleted = true; },
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
