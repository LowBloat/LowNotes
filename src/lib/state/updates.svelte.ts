import { getUpdatePolicy } from '$lib/api';
import { appAlert } from '$lib/dialogs.svelte';
import { ts } from '$lib/i18n';
import { checkAvailableUpdate, type AvailableUpdate } from '$lib/updates';
import type { AppSettings, UpdatePolicy } from '$lib/types';

const CHECK_INTERVAL_MS = 10 * 60 * 1000;
/** Focus comes and goes quickly; at most one extra check per this long. */
const FOCUS_GAP_MS = 2 * 60 * 1000;

/** Looks for a new version at startup, every few minutes and when
 * the window regains focus, honoring the user's automatic-check preference and
 * a skipped version. A copy that can update itself (Windows installer, macOS
 * app, writable AppImage, .deb or .rpm) downloads the signed update in the
 * background first, so the dialog only has to restart to install it. Each
 * version is announced once; the Help menu check opens it again. */
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
    if (busy) {
      if (manual && state.pending) state.open = true;
      return;
    }
    busy = true;
    lastCheck = Date.now();
    try {
      state.policy = await getUpdatePolicy();
      const update = await checkAvailableUpdate(state.policy);
      if (!update) {
        state.pending = null;
        if (manual) void appAlert(ts('titlebar.upToDate'));
        return;
      }
      // Already announced (and maybe downloaded): keep it, the download lives
      // in that installer. Only an explicit check opens the dialog again.
      if (state.pending?.version === update.version) {
        if (manual) state.open = true;
        return;
      }
      if (!manual && update.version === settings?.skipped_version) return;
      if (!manual && update.installer) {
        await prefetch(update);
      } else {
        state.pending = update;
      }
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
      await update.installer?.download();
      state.pending = { ...update, downloaded: true };
    } catch (error) {
      console.error('Failed to download the update in the background:', error);
      state.pending = update;
    }
  }

  function onFocus() {
    if (Date.now() - lastCheck > FOCUS_GAP_MS) void check();
  }

  function start() {
    void check();
    timer = setInterval(() => void check(), CHECK_INTERVAL_MS);
    window.addEventListener('focus', onFocus);
  }

  function stop() {
    if (timer) clearInterval(timer);
    window.removeEventListener('focus', onFocus);
  }

  return { state, check, start, stop };
}
