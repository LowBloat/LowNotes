import { getUpdatePolicy } from '$lib/api';
import { appAlert } from '$lib/dialogs.svelte';
import { ts } from '$lib/i18n';
import { checkAvailableUpdate, type AvailableUpdate } from '$lib/updates';
import type { AppSettings, UpdatePolicy } from '$lib/types';

const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

/** Looks for a new version at startup and every six hours, honoring the
 * user's automatic-check preference and a skipped version. */
export function createUpdateChecker(getSettings: () => AppSettings | null) {
  const state = $state({
    pending: null as AvailableUpdate | null,
    policy: null as UpdatePolicy | null,
    open: false,
  });
  let timer: ReturnType<typeof setInterval> | undefined;

  /** A manual check (Help menu) ignores the automatic check preference and a
   * skipped version, and reports when there is nothing new. */
  async function check(manual = false) {
    const settings = getSettings();
    if (!manual && settings?.update_check === false) return;
    try {
      state.policy = await getUpdatePolicy();
      const update = await checkAvailableUpdate(state.policy);
      if (update && (manual || update.version !== settings?.skipped_version)) {
        state.pending = update;
        state.open = true;
      } else if (manual) {
        void appAlert(ts('titlebar.upToDate'));
      }
    } catch (error) {
      console.error('Failed to check for updates:', error);
      if (manual) void appAlert(ts('titlebar.updateCheckFailed'));
    }
  }

  function start() {
    void check();
    timer = setInterval(() => void check(), CHECK_INTERVAL_MS);
  }

  function stop() {
    if (timer) clearInterval(timer);
  }

  return { state, check, start, stop };
}
