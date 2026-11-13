import { expect, test } from 'bun:test';
import { collapsedFolderSaver, renameCollapsedFolders } from '../src/lib/note-tree';

test('renaming a folder keeps it and its subfolders collapsed', () => {
  expect(renameCollapsedFolders(['Python', 'Python/Etapas', 'Pythonic', 'Other'], 'Python', 'Code/Python'))
    .toEqual(['Code/Python', 'Code/Python/Etapas', 'Pythonic', 'Other']);
});

test('collapsed folders are saved one call at a time, keeping the latest state per vault', async () => {
  const calls: Array<[string, string[]]> = [];
  let release!: () => void;
  const firstSave = new Promise<void>((resolve) => (release = resolve));
  const remember = collapsedFolderSaver(async (vaultId, folders) => {
    calls.push([vaultId, folders]);
    if (calls.length === 1) await firstSave;
  });

  const done = remember('a', ['Projects']);
  void remember('a', ['Projects', 'Journal']);
  void remember('b', ['Inbox']);
  void remember('a', ['Journal']);
  expect(calls).toEqual([['a', ['Projects']]]);
  release();
  await done;
  expect(calls).toEqual([['a', ['Projects']], ['a', ['Journal']], ['b', ['Inbox']]]);
});

test('a failed save does not stop the next one', async () => {
  const calls: string[][] = [];
  const originalError = console.error;
  console.error = () => {};
  try {
    const remember = collapsedFolderSaver(async (_vaultId, folders) => {
      calls.push(folders);
      if (calls.length === 1) throw new Error('disk full');
    });
    await remember('a', ['One']);
    await remember('a', ['Two', 'One']);
  } finally {
    console.error = originalError;
  }
  expect(calls).toEqual([['One'], ['One', 'Two']]);
});
