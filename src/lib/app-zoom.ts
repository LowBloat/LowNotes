const STORAGE_KEY = 'lownotes:app-zoom';
const MIN_ZOOM = 0.5;
const MAX_ZOOM = 2;
const ZOOM_STEP = 0.1;

function clampZoom(value: number): number {
  return Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, Math.round(value * 10) / 10));
}

let stepZoom: ((direction: -1 | 0 | 1) => void) | null = null;

/** Zooms like the shortcuts do: in (1), out (-1) or back to 100% (0). */
export function changeAppZoom(direction: -1 | 0 | 1): void {
  stepZoom?.(direction);
}

/** Install app-wide zoom shortcuts in the desktop webview. */
export function installAppZoom(
  target: Window,
  applyZoom: (factor: number) => Promise<void>,
): () => void {
  let zoom = 1;
  let appliedZoom = 1;
  let disposed = false;
  let pending = Promise.resolve();

  try {
    const stored = Number(target.localStorage.getItem(STORAGE_KEY));
    if (Number.isFinite(stored) && stored > 0) zoom = clampZoom(stored);
  } catch {
    // Shortcuts still work when storage is unavailable.
  }

  function setZoom(value: number) {
    const next = clampZoom(value);
    zoom = next;
    // Preserve wheel/keyboard order when several IPC calls are in flight.
    pending = pending.then(async () => {
      if (disposed) return;
      try {
        await applyZoom(next);
        appliedZoom = next;
        try { target.localStorage.setItem(STORAGE_KEY, String(next)); } catch { /* optional */ }
      } catch (error) {
        if (zoom === next) zoom = appliedZoom;
        console.error('Failed to apply app zoom:', error);
      }
    });
  }

  function handleKeydown(event: KeyboardEvent) {
    if (!(event.ctrlKey || event.metaKey) || event.altKey || event.isComposing) return;
    let next: number;
    if (event.key === '+' || event.key === '=' || event.code === 'NumpadAdd') {
      next = zoom + ZOOM_STEP;
    } else if (event.key === '-' || event.key === '_' || event.code === 'NumpadSubtract') {
      next = zoom - ZOOM_STEP;
    } else if (event.key === '0' || event.code === 'Numpad0') {
      next = 1;
    } else return;
    event.preventDefault();
    event.stopPropagation();
    if (clampZoom(next) !== zoom) setZoom(next);
  }

  function handleWheel(event: WheelEvent) {
    if (!event.ctrlKey || event.altKey || event.deltaY === 0) return;
    event.preventDefault();
    event.stopPropagation();
    const next = clampZoom(zoom + (event.deltaY < 0 ? ZOOM_STEP : -ZOOM_STEP));
    if (next !== zoom) setZoom(next);
  }

  setZoom(zoom);
  target.addEventListener('keydown', handleKeydown, true);
  target.addEventListener('wheel', handleWheel, { capture: true, passive: false });
  const step = (direction: -1 | 0 | 1) => {
    const next = clampZoom(direction === 0 ? 1 : zoom + direction * ZOOM_STEP);
    if (next !== zoom) setZoom(next);
  };
  stepZoom = step;
  return () => {
    disposed = true;
    if (stepZoom === step) stepZoom = null;
    target.removeEventListener('keydown', handleKeydown, true);
    target.removeEventListener('wheel', handleWheel, true);
  };
}
