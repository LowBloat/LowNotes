/** Themed right-click menus. Components describe the entries for what was
 * clicked; `ContextMenu.svelte`, mounted once by the page, shows them. */

import { cleanEntries, type MenuEntries, type MenuEntry } from './menu';

export interface ContextMenuRequest {
  x: number;
  y: number;
  entries: MenuEntry[];
  /** Focused when the menu closes. */
  returnFocus: HTMLElement | null;
}

export const contextMenu = $state<{ current: ContextMenuRequest | null }>({ current: null });

/** Shows `entries` for a `contextmenu` event instead of the WebView's menu. A
 * menu opened from the keyboard appears below the focused element. */
export function openContextMenu(event: MouseEvent, entries: MenuEntries): void {
  event.preventDefault();
  event.stopPropagation();
  const clean = cleanEntries(entries);
  if (!clean.length) return;
  let { clientX: x, clientY: y } = event;
  const target = event.currentTarget instanceof HTMLElement ? event.currentTarget : null;
  if (x === 0 && y === 0 && target) {
    const rect = target.getBoundingClientRect();
    x = rect.left + 8;
    y = rect.bottom;
  }
  const active = document.activeElement;
  contextMenu.current = { x, y, entries: clean, returnFocus: active instanceof HTMLElement ? active : null };
}

export function closeContextMenu(restoreFocus = false): void {
  const request = contextMenu.current;
  contextMenu.current = null;
  if (restoreFocus) request?.returnFocus?.focus();
}
