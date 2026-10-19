import { takeRecoveryNotices } from '$lib/api';

export interface RecoveryNotice { path: string; recovered: boolean }

/** Files restored after an interrupted save, polled from the native side. */
export function createRecoveryNotices() {
  const state = $state({ notices: [] as RecoveryNotice[] });
  const seen = new Set<string>();
  let timer: ReturnType<typeof setInterval> | undefined;

  async function refresh() {
    try {
      for (const notice of await takeRecoveryNotices()) {
        const key = `${notice.path}:${notice.recovered}`;
        if (!seen.has(key)) {
          seen.add(key);
          state.notices.push(notice);
        }
      }
    } catch (error) {
      console.error('Failed to read recovery notices:', error);
    }
  }

  function start() {
    timer = setInterval(() => void refresh(), 3000);
  }

  function stop() {
    if (timer) clearInterval(timer);
  }

  function dismiss() {
    state.notices = [];
  }

  return { state, refresh, start, stop, dismiss };
}
