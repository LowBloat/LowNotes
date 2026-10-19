import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { networkGetPairInfo } from '$lib/api';
import { trError } from '$lib/i18n';
import type { NetworkEventPayload, PairInfo, PeerConfig } from '$lib/types';

export type SyncStatus = 'idle' | 'syncing' | 'synced' | 'error';
export interface ConflictNotice { note_path: string; conflict_path: string }
export interface PairRequest { request_id: string; peer: PeerConfig }

/** P2P state shown by the shell: this device's pair code, sync status,
 * conflict copies waiting for review and an incoming pairing request. */
export function createP2pSession(hooks: {
  /** The vault listing may have changed (sync finished, conflict copy created). */
  onItemsChanged: () => void;
  /** A paired device approved our request or we approved theirs. */
  onPeerApproved: (peer: PeerConfig) => void;
}) {
  const state = $state({
    pairCode: '',
    endpointId: '',
    syncStatus: 'idle' as SyncStatus,
    conflicts: [] as ConflictNotice[],
    incomingRequest: null as PairRequest | null,
  });
  let unlisteners: UnlistenFn[] = [];
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  let remoteEditTimer: ReturnType<typeof setTimeout> | undefined;

  function setPairInfo(info: PairInfo | null | undefined) {
    state.pairCode = info?.pair_code ?? '';
    state.endpointId = info?.endpoint_id ?? '';
  }

  async function refreshPairInfo() {
    try {
      const info = await networkGetPairInfo();
      if (info) setPairInfo(info);
    } catch (error) {
      console.error('Failed to fetch P2P code:', error);
    }
  }

  async function start() {
    // Register every listener before loading data so no event is lost.
    unlisteners = await Promise.all([
      listen<NetworkEventPayload>('p2p:ready', (event) => {
        if (event.payload.type === 'Ready') setPairInfo({ pair_code: event.payload.pair_code, endpoint_id: event.payload.endpoint_id });
      }),
      listen<NetworkEventPayload>('p2p:syncing', () => {
        state.syncStatus = 'syncing';
      }),
      listen<NetworkEventPayload>('p2p:synced', () => {
        state.syncStatus = 'synced';
        hooks.onItemsChanged();
        if (idleTimer) clearTimeout(idleTimer);
        idleTimer = setTimeout(() => { state.syncStatus = 'idle'; }, 3000);
      }),
      listen<NetworkEventPayload>('p2p:pair-requested', (event) => {
        if (event.payload.type === 'PairRequested') {
          state.incomingRequest = { request_id: event.payload.request_id, peer: event.payload.peer };
        }
      }),
      listen<{ type: string; peer: PeerConfig }>('p2p:pair-approved', (event) => {
        if (event.payload.peer) hooks.onPeerApproved(event.payload.peer);
      }),
      listen<NetworkEventPayload>('p2p:error', (event) => {
        if (event.payload.type === 'Error') {
          state.syncStatus = 'error';
          console.error('[p2p]', trError(event.payload.message), event.payload.peer ?? '');
        }
      }),
      // Remote typing rarely changes the listing; titles can, so refresh lazily.
      listen<NetworkEventPayload>('p2p:crdt-update', () => {
        if (remoteEditTimer) clearTimeout(remoteEditTimer);
        remoteEditTimer = setTimeout(hooks.onItemsChanged, 1500);
      }),
      listen<NetworkEventPayload>('p2p:conflict', (event) => {
        if (event.payload.type !== 'Conflict') return;
        const notice = { note_path: event.payload.note_path, conflict_path: event.payload.conflict_path };
        // Keep every pending conflict; a second one must not hide the first.
        if (!state.conflicts.some((item) => item.conflict_path === notice.conflict_path)) state.conflicts.push(notice);
        hooks.onItemsChanged();
      }),
    ]);
  }

  function dismissConflict(conflictPath: string) {
    state.conflicts = state.conflicts.filter((item) => item.conflict_path !== conflictPath);
  }

  function stop() {
    unlisteners.forEach((stopListening) => stopListening());
    unlisteners = [];
    if (idleTimer) clearTimeout(idleTimer);
    if (remoteEditTimer) clearTimeout(remoteEditTimer);
  }

  return { state, start, stop, setPairInfo, refreshPairInfo, dismissConflict };
}
