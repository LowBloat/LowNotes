/** Entries shared by the right-click menus and the title bar menus, both
 * shown by `MenuList.svelte`. */

import type { Icon } from 'lucide-svelte';

export interface MenuItem {
  label: string;
  icon?: typeof Icon;
  /** Shortcut hint shown on the right; the shortcut itself lives elsewhere. */
  shortcut?: string;
  disabled?: boolean;
  danger?: boolean;
  /** Shown as a check box when defined. */
  checked?: boolean;
  action: () => unknown;
}

export type MenuEntry = MenuItem | { separator: true };

/** Entries may be left out with `false`, `null` or `undefined`. */
export type MenuEntries = Array<MenuEntry | false | null | undefined>;

export const SEPARATOR: MenuEntry = { separator: true };

export function isSeparator(entry: MenuEntry): entry is { separator: true } {
  return 'separator' in entry;
}

/** Drops omitted entries and separators at the edges or next to each other. */
export function cleanEntries(entries: MenuEntries): MenuEntry[] {
  const out: MenuEntry[] = [];
  for (const entry of entries) {
    if (!entry) continue;
    if (isSeparator(entry) && (!out.length || isSeparator(out[out.length - 1]))) continue;
    out.push(entry);
  }
  while (out.length && isSeparator(out[out.length - 1])) out.pop();
  return out;
}

/** Text fields keep the WebView's menu for spelling suggestions; everywhere
 * else it only offers browser actions such as Reload, which would restart the
 * interface. */
export function keepsNativeMenu(target: EventTarget | null): boolean {
  return typeof Element !== 'undefined' && target instanceof Element && !!target.closest('input, textarea, select');
}

/** Shortcut hint for the platform: "Ctrl+F" reads "⌘F" on macOS. */
export function shortcutLabel(keys: string, mac = typeof navigator !== 'undefined' && /Mac/.test(navigator.platform)): string {
  return mac ? keys.replace('Ctrl+', '⌘').replace('Shift+', '⇧').replace('Alt+', '⌥') : keys;
}

/** A wiki link that resolves to `path` from any note: the vault path without
 * the Markdown extension. */
export function wikiLinkFor(path: string): string {
  return `[[${path.replace(/\.(?:md|markdown)$/i, '')}]]`;
}

/** Default name offered when duplicating `path`: "Ideas.md" becomes
 * "Ideas copy.md" in the same folder. */
export function duplicateName(path: string, suffix: string): string {
  const match = /^(.*?)(\.(?:md|markdown))?$/i.exec(path);
  return `${match?.[1] ?? path} ${suffix}${match?.[2] ?? ''}`;
}
