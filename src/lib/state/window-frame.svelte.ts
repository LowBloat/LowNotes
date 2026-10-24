import { getCurrentWindow } from '@tauri-apps/api/window';
import type { UnlistenFn } from '@tauri-apps/api/event';

/** The window frame drawn by the title bar: maximized state for the
 * maximize/restore button and the window actions behind its buttons. */
export function createWindowFrame() {
  const state = $state({ maximized: false, focused: true });
  const unlisteners: UnlistenFn[] = [];
  let stopped = false;

  // Outside the desktop app (the plain web build) there is no window to drive.
  function current() {
    try {
      return getCurrentWindow();
    } catch {
      return null;
    }
  }

  async function refresh() {
    const window = current();
    if (!window) return;
    try {
      state.maximized = await window.isMaximized();
    } catch {
      // The frame keeps its last known state.
    }
  }

  async function start() {
    const window = current();
    if (!window) return;
    try {
      const listeners = await Promise.all([
        window.onResized(() => void refresh()),
        window.onFocusChanged(({ payload }) => { state.focused = payload; }),
      ]);
      if (stopped) listeners.forEach((unlisten) => unlisten());
      else unlisteners.push(...listeners);
    } catch {
      // Without events the buttons still work; only the icons may lag.
    }
    await refresh();
  }

  function stop() {
    stopped = true;
    unlisteners.splice(0).forEach((unlisten) => unlisten());
  }

  return {
    state,
    start,
    stop,
    minimize: () => void current()?.minimize().catch(() => {}),
    toggleMaximize: () => void current()?.toggleMaximize().then(refresh, () => {}),
    /** Closing honors "close to tray" like the native button did. */
    close: () => void current()?.close().catch(() => {}),
  };
}
