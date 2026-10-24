/** The application menus (File, Edit, View, Help). Windows and Linux show
 * them in the title bar; macOS puts them in the global menu bar. */

import { cleanEntries, isSeparator, type MenuEntries, type MenuEntry } from './menu';

/** Edit menu commands the editor runs for the page. */
export type EditorCommand = 'undo' | 'redo' | 'cut' | 'copy' | 'paste' | 'selectAll' | 'find' | 'replace';

export interface AppMenu {
  id: string;
  label: string;
  entries: MenuEntry[];
}

export function appMenu(id: string, label: string, entries: MenuEntries): AppMenu {
  return { id, label, entries: cleanEntries(entries) };
}

/** Native accelerator for a shortcut hint: "Ctrl+Shift+Z" is Command+Shift+Z
 * on macOS. */
export function accelerator(shortcut: string | undefined): string | undefined {
  return shortcut?.replace(/^Ctrl\+/, 'CmdOrCtrl+');
}

/** Replaces the macOS menu bar with `menus`, after an application menu with
 * About, Settings, Hide and Quit, and before the standard Window menu. Edit
 * entries marked `native` stay predefined items, so Command+C and friends keep
 * working in every text field. */
export async function setNativeAppMenu(menus: AppMenu[], app: { name: string; about: MenuEntry; settings: MenuEntry; quit: string; window: string }): Promise<void> {
  const { Menu, Submenu, MenuItem, CheckMenuItem, PredefinedMenuItem } = await import('@tauri-apps/api/menu');
  const item = (entry: MenuEntry) => {
    if (isSeparator(entry)) return PredefinedMenuItem.new({ item: 'Separator' });
    if (entry.native) return PredefinedMenuItem.new({ item: entry.native, text: entry.label });
    const options = { text: entry.label, enabled: !entry.disabled, accelerator: accelerator(entry.shortcut), action: () => void entry.action() };
    return entry.checked === undefined ? MenuItem.new(options) : CheckMenuItem.new({ ...options, checked: entry.checked });
  };
  const submenu = async (text: string, entries: MenuEntry[]) => Submenu.new({ text, items: await Promise.all(entries.map(item)) });
  const separator: MenuEntry = { separator: true };
  const menu = await Menu.new({
    items: await Promise.all([
      submenu(app.name, [app.about, separator, app.settings, separator]).then(async (application) => {
        await application.append(await Promise.all([
          PredefinedMenuItem.new({ item: 'Services' }), PredefinedMenuItem.new({ item: 'Separator' }),
          PredefinedMenuItem.new({ item: 'Hide' }), PredefinedMenuItem.new({ item: 'HideOthers' }), PredefinedMenuItem.new({ item: 'ShowAll' }),
          PredefinedMenuItem.new({ item: 'Separator' }), PredefinedMenuItem.new({ item: 'Quit', text: app.quit }),
        ]));
        return application;
      }),
      ...menus.map((menu) => submenu(menu.label, menu.entries)),
      Submenu.new({ text: app.window, items: await Promise.all([
        PredefinedMenuItem.new({ item: 'Minimize' }), PredefinedMenuItem.new({ item: 'Maximize' }),
        PredefinedMenuItem.new({ item: 'Fullscreen' }), PredefinedMenuItem.new({ item: 'Separator' }),
        PredefinedMenuItem.new({ item: 'BringAllToFront' }),
      ]) }),
    ]),
  });
  const previous = await menu.setAsAppMenu();
  await previous?.close();
}
