import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import * as Y from 'yjs';
import { Awareness, applyAwarenessUpdate, outdatedTimeout, removeAwarenessStates } from 'y-protocols/awareness';
import { parsePresenceState, type PresenceUser } from '$lib/presence';

/** Devices editing anywhere in the vault, for the sidebar. The open editor
 * keeps its own awareness, attached to its document, for remote cursors. */
export function createPresenceTracker() {
  const state = $state({ editors: [] as PresenceUser[] });
  let doc: Y.Doc | null = null;
  let registry: Awareness | null = null;
  let unlisten: UnlistenFn | null = null;
  let pruneTimer: ReturnType<typeof setInterval> | undefined;

  function refresh() {
    if (!registry) {
      state.editors = [];
      return;
    }
    const byDevice = new Map<string, PresenceUser>();
    for (const [clientId, value] of registry.getStates()) {
      if (clientId === registry.clientID) continue;
      const user = parsePresenceState(value)?.user;
      if (user?.deviceId) byDevice.set(user.deviceId, user);
    }
    state.editors = [...byDevice.values()];
  }

  function prune() {
    if (!registry) return;
    const now = Date.now();
    const stale: number[] = [];
    registry.meta.forEach((meta, clientId) => {
      if (clientId !== registry!.clientID && now - meta.lastUpdated > outdatedTimeout) stale.push(clientId);
    });
    if (stale.length > 0) {
      removeAwarenessStates(registry, stale, 'prune');
      refresh();
    }
  }

  async function start() {
    doc = new Y.Doc();
    registry = new Awareness(doc);
    unlisten = await listen<{ note_path: string; update: number[] }>('p2p:awareness', (event) => {
      if (!registry) return;
      applyAwarenessUpdate(registry, new Uint8Array(event.payload.update), 'remote');
      refresh();
    });
    pruneTimer = setInterval(prune, 15000);
  }

  function stop() {
    unlisten?.();
    unlisten = null;
    if (pruneTimer) clearInterval(pruneTimer);
    registry?.destroy();
    registry = null;
    doc?.destroy();
    doc = null;
  }

  return {
    get editors() { return state.editors; },
    start,
    stop,
  };
}
