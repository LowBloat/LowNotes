import { expect, test } from 'bun:test';
import { EditorState, Text } from '@codemirror/state';
import { markdown } from '@codemirror/lang-markdown';
import { SearchQuery } from '@codemirror/search';
import { combineSearchMatches, noteSearchHighlights, searchMatches, setNoteSearchHighlights, type PreviewMatch } from '../src/lib/note-search';
import { renderMarkdown } from '../src/lib/markdown';

test('find supports case, whole words, regex, Unicode and invalid or empty patterns', () => {
  const text = Text.of(['café Café cafeteria café2', 'café']);
  const count = (search: string, options = {}) => searchMatches(text, new SearchQuery({ search, ...options })).length;
  expect(count('CAFÉ')).toBe(4);
  expect(count('café', { caseSensitive: true, wholeWord: true })).toBe(2);
  expect(count('café\\d', { regexp: true })).toBe(1);
  expect(count('[', { regexp: true })).toBe(0);
  expect(count('^', { regexp: true })).toBe(0);
  expect(count('')).toBe(0);
});

function rendered(text: string, from: number, to: number): PreviewMatch {
  return { text, from: 0, to: text.length, sourceFrom: from, sourceTo: to, parts: [] };
}

test('split search pairs repeated visible occurrences with their own Markdown blocks, without counting twice', () => {
  const doc = '# needle\n\n[needle](https://example.org/needle)\n\n- [ ] needle';
  const state = EditorState.create({ doc, extensions: [markdown()] });
  const source = searchMatches(state.doc, new SearchQuery({ search: 'needle' }));
  const preview = [rendered('needle', 0, 9), rendered('needle', 10, 47), rendered('needle', 48, doc.length)];
  const combined = combineSearchMatches(state, source, preview, 'split');
  expect(combined).toHaveLength(4);
  expect(combined.map(match => match.previewIndex)).toEqual([0, 1, -1, 2]);
  expect(combineSearchMatches(state, source, preview, 'preview').map(match => match.sourceIndex)).toEqual([0, 1, 3]);
  expect(combineSearchMatches(state, source, preview, 'edit').map(match => match.previewIndex)).toEqual([-1, -1, -1, -1]);
});

test('rendered phrases across Markdown formatting remain searchable and point to their source block', () => {
  const doc = '**hello** world\n\nhello world';
  const state = EditorState.create({ doc, extensions: [markdown()] });
  const source = searchMatches(state.doc, new SearchQuery({ search: 'hello world' }));
  const preview = [rendered('hello world', 0, 16), rendered('hello world', 17, doc.length)];
  const results = combineSearchMatches(state, source, preview, 'split');
  expect(results).toHaveLength(2);
  expect(results[0]).toEqual({ from: 0, to: 16, sourceIndex: -1, previewIndex: 0 });
  expect(results[1].sourceIndex).toBe(0);
});

test('search decorations follow edits and clear without changing note content', () => {
  let state = EditorState.create({ doc: 'hello hello', extensions: [noteSearchHighlights] });
  state = state.update({ effects: setNoteSearchHighlights.of({ matches: [{ from: 6, to: 11 }], active: 0 }) }).state;
  state = state.update({ changes: { from: 0, insert: '🙂' } }).state;
  const cursor = state.field(noteSearchHighlights).iter();
  expect([cursor.from, cursor.to, cursor.value?.spec.class]).toEqual([8, 13, 'note-search-match note-search-current']);
  state = state.update({ effects: setNoteSearchHighlights.of({ matches: [], active: -1 }) }).state;
  expect(state.field(noteSearchHighlights).size).toBe(0);
  expect(state.doc.toString()).toBe('🙂hello hello');
});

test('preview source maps preserve original CRLF and Unicode offsets, task controls and code blocks', () => {
  const source = '# 🙂 title\r\n\r\n- [ ] Task\r\n\r\n```md\r\nneedle\r\n```';
  const html = renderMarkdown(source, undefined, { sourceMap: true, interactiveTasks: true });
  expect(html).toContain(`data-search-from="${source.indexOf('- [ ]')}"`);
  expect(html).toContain(`data-task-offset="${source.indexOf('[ ]') + 1}"`);
  expect(html).toContain(`data-search-from="${source.indexOf('```')}"`);
  expect(html).toContain(`data-search-to="${source.length}"`);
  expect(html).not.toContain('disabled');
  expect(renderMarkdown(source)).not.toContain('data-search-');
  const normalized = source.replace(/\r\n/g, '\n');
  const editorHtml = renderMarkdown(source, undefined, { sourceMap: 'editor' });
  expect(editorHtml).toContain(`data-search-from="${normalized.indexOf('- [ ]')}"`);
  expect(editorHtml).toContain(`data-search-to="${normalized.length}"`);
  const table = 'Intro\n\n| Cell | Value |\n|---|---|\n| needle | result |';
  expect(renderMarkdown(table, undefined, { sourceMap: true })).toContain(`data-search-from="${table.indexOf('| needle')}"`);
});
