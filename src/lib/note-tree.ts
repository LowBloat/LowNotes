import type { VaultItem } from './types';

export interface NoteTreeRow {
  item: VaultItem;
  depth: number;
}

/** Keep the vault hierarchy visible while filtering notes and their ancestors. */
export function visibleNoteRows(items: VaultItem[], collapsed: ReadonlySet<string>, query = ''): NoteTreeRow[] {
  const byParent = new Map<string, VaultItem[]>();
  for (const item of items) {
    const parent = item.path.includes('/') ? item.path.slice(0, item.path.lastIndexOf('/')) : '';
    const siblings = byParent.get(parent) ?? [];
    siblings.push(item);
    byParent.set(parent, siblings);
  }
  for (const siblings of byParent.values()) {
    siblings.sort((a, b) => Number(b.is_dir) - Number(a.is_dir)
      || a.name.localeCompare(b.name, undefined, { sensitivity: 'base', numeric: true }));
  }

  const needle = query.trim().toLocaleLowerCase();
  const matches = new Set<string>();
  if (needle) {
    for (const item of items) {
      if (!item.title.toLocaleLowerCase().includes(needle)
        && !item.path.toLocaleLowerCase().includes(needle)) continue;
      let path = item.path;
      while (path) {
        matches.add(path);
        path = path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
      }
    }
  }

  const rows: NoteTreeRow[] = [];
  const visit = (parent: string, depth: number) => {
    for (const item of byParent.get(parent) ?? []) {
      if (needle && !matches.has(item.path)) continue;
      rows.push({ item, depth });
      if (item.is_dir && (needle || !collapsed.has(item.path))) visit(item.path, depth + 1);
    }
  };
  visit('', 0);
  return rows;
}

/** Collapsed folders after `oldPath` was renamed to `newPath`, subfolders included. */
export function renameCollapsedFolders(collapsed: Iterable<string>, oldPath: string, newPath: string): string[] {
  return [...collapsed].map((path) => path === oldPath ? newPath
    : path.startsWith(`${oldPath}/`) ? `${newPath}${path.slice(oldPath.length)}` : path);
}

/** Saves the collapsed folders of each vault one call at a time; folders
 * toggled while a save runs are saved next, and only their latest state. */
export function collapsedFolderSaver(save: (vaultId: string, folders: string[]) => Promise<void>) {
  const queued = new Map<string, string[]>();
  let running: Promise<void> | null = null;
  async function drain() {
    for (let next = queued.entries().next(); !next.done; next = queued.entries().next()) {
      const [vaultId, folders] = next.value;
      queued.delete(vaultId);
      try {
        await save(vaultId, folders);
      } catch (error) {
        console.error('Failed to remember the collapsed folders:', error);
      }
    }
    running = null;
  }
  return (vaultId: string, folders: Iterable<string>): Promise<void> => {
    queued.set(vaultId, [...folders].sort());
    running ??= drain();
    return running;
  };
}
