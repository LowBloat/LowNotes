import { getUpdatePolicy } from '$lib/api';
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

  async function check() {
    const settings = getSettings();
    if (settings?.update_check === false) return;
    try {
      state.policy = await getUpdatePolicy();
      const update = await checkAvailableUpdate(state.policy);
      if (update && update.version !== settings?.skipped_version) {
        state.pending = update;
        state.open = true;
      }
    } catch (error) {
      console.error('Failed to check for updates:', error);
    }
  }

  function start() {
    void check();
    timer = setInterval(check, CHECK_INTERVAL_MS);
  }

  function stop() {
    if (timer) clearInterval(timer);
  }

  return { state, check, start, stop };
}
