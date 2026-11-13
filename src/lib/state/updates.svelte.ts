import { getUpdatePolicy } from '$lib/api';
import { appAlert } from '$lib/dialogs.svelte';
import { ts } from '$lib/i18n';
import { checkAvailableUpdate, discardUpdate, type AvailableUpdate } from '$lib/updates';
import type { AppSettings, UpdatePolicy } from '$lib/types';

/** The manifest is a small file on GitHub; checking every minute means a new
 * release reaches open windows within a minute of being published. */
const CHECK_INTERVAL_MS = 60 * 1000;
/** Focus comes and goes quickly; at most one extra check per this long. */
const FOCUS_GAP_MS = 15 * 1000;
/** A stalled background download must not block later checks forever. The
 * dialog's own download has no limit. */
const DOWNLOAD_TIMEOUT_MS = 15 * 60 * 1000;

/** Looks for a new version at startup, every minute and when the window
 * comes back, honoring the user's automatic-check preference and a skipped
 * version. A copy that can update itself (Windows installer, macOS app,
 * writable AppImage, .deb or .rpm) downloads the signed update in the
 * background first, so the dialog only has to restart to install it. Each
 * version opens the dialog once; afterwards the title bar keeps offering it,
 * and a newer release replaces it as soon as it is published. */
export function createUpdateChecker(getSettings: () => AppSettings | null) {
  const state = $state({
    pending: null as AvailableUpdate | null,
    policy: null as UpdatePolicy | null,
    open: false,
  });
  let timer: ReturnType<typeof setInterval> | undefined;
  let busy = false;
  let lastCheck = 0;

  /** A manual check (Help menu) ignores the automatic check preference and a
   * skipped version, and reports when there is nothing new. */
  async function check(manual = false) {
    const settings = getSettings();
    if (!manual && settings?.update_check === false) return;
    // The dialog shows one version at a time; look again once it closes.
    if (busy || (state.open && !manual)) {
      if (manual && state.pending) state.open = true;
      return;
    }
    busy = true;
    lastCheck = Date.now();
    try {
      state.policy ??= await getUpdatePolicy();
      const update = await checkAvailableUpdate(state.policy);
      if (!update) {
        discardUpdate(state.pending);
        state.pending = null;
        if (manual) void appAlert(ts('titlebar.upToDate'));
        return;
      }
      // Already announced (and maybe downloaded): keep it, the download lives
      // in that installer. Only an explicit check opens the dialog again.
      if (state.pending?.version === update.version) {
        discardUpdate(update);
        if (manual) state.open = true;
        return;
      }
      if (!manual && update.version === settings?.skipped_version) {
        discardUpdate(update);
        return;
      }
      const previous = state.pending;
      if (!manual && update.installer) {
        await prefetch(update);
      } else {
        state.pending = update;
      }
      discardUpdate(previous);
      state.open = true;
    } catch (error) {
      console.error('Failed to check for updates:', error);
      if (manual) void appAlert(ts('titlebar.updateCheckFailed'));
    } finally {
      busy = false;
    }
  }

  /** Downloads and verifies quietly; on failure the dialog downloads again. */
  async function prefetch(update: AvailableUpdate) {
    try {
      await update.installer?.download(undefined, { timeout: DOWNLOAD_TIMEOUT_MS });
      state.pending = { ...update, downloaded: true };
    } catch (error) {
      console.error('Failed to download the update in the background:', error);
      state.pending = update;
    }
  }

  function checkSoon() {
    if (Date.now() - lastCheck > FOCUS_GAP_MS) void check();
  }

  // The window can come back from the tray or another desktop without focus.
  function onVisibilityChange() {
    if (document.visibilityState === 'visible') checkSoon();
  }

  function start() {
    if (timer) return;
    void check();
    timer = setInterval(() => void check(), CHECK_INTERVAL_MS);
    window.addEventListener('focus', checkSoon);
    document.addEventListener('visibilitychange', onVisibilityChange);
  }

  function stop() {
    if (timer) clearInterval(timer);
    timer = undefined;
    window.removeEventListener('focus', checkSoon);
    document.removeEventListener('visibilitychange', onVisibilityChange);
  }

  return { state, check, start, stop };
}
