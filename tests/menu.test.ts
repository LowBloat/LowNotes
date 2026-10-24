import { expect, test } from 'bun:test';
import { SEPARATOR, cleanEntries, duplicateName, keepsNativeMenu, shortcutLabel, wikiLinkFor } from '../src/lib/menu';

const item = (label: string) => ({ label, action: () => {} });

test('menus drop omitted entries and stray separators', () => {
  const entries = cleanEntries([SEPARATOR, item('Open'), false, SEPARATOR, null, SEPARATOR, item('Delete'), undefined, SEPARATOR]);
  expect(entries.map((entry) => ('label' in entry ? entry.label : '-'))).toEqual(['Open', '-', 'Delete']);
  expect(cleanEntries([SEPARATOR, false])).toEqual([]);
});

test('copied links resolve to the note path from any folder', () => {
  expect(wikiLinkFor('Ideas.md')).toBe('[[Ideas]]');
  expect(wikiLinkFor('Projects/2026/plan.markdown')).toBe('[[Projects/2026/plan]]');
});

test('a duplicate keeps its folder and extension', () => {
  expect(duplicateName('Ideas.md', 'copy')).toBe('Ideas copy.md');
  expect(duplicateName('Projects/plan.markdown', 'cópia')).toBe('Projects/plan cópia.markdown');
  expect(duplicateName('notes', 'copy')).toBe('notes copy');
});

test('shortcut hints follow the platform', () => {
  expect(shortcutLabel('Ctrl+F', false)).toBe('Ctrl+F');
  expect(shortcutLabel('Ctrl+Shift+Z', true)).toBe('⌘⇧Z');
});

test('only text fields keep the WebView menu', () => {
  expect(keepsNativeMenu(null)).toBe(false);
});
