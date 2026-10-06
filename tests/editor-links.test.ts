import { expect, test } from 'bun:test';
import { EditorState } from '@codemirror/state';
import { markdown } from '@codemirror/lang-markdown';
import { editorLinkAt } from '../src/lib/editor-links';

function link(source: string, target: string) {
  const state = EditorState.create({ doc: source, extensions: [markdown()] });
  return editorLinkAt(state, source.indexOf(target) + Math.floor(target.length / 2));
}

test('opens Markdown labels, image URLs and autolinks, decoding escaped destinations', () => {
  const source = '[Docs](https://example.org/docs "Title") ![Photo](https://example.org/p.png) <https://example.org/auto>';
  expect(link(source, 'Docs')).toBe('https://example.org/docs');
  expect(link(source, 'example.org/docs')).toBe('https://example.org/docs');
  expect(link(source, 'Photo')).toBe('https://example.org/p.png');
  expect(link(source, 'example.org/auto')).toBe('https://example.org/auto');
  expect(link('[A](https://example.org/a\\(b\\)?a=1&amp;b=2)', 'A')).toBe('https://example.org/a(b)?a=1&b=2');
});

test('resolves full, collapsed, shortcut and image references using Markdown definitions', () => {
  const source = '[Docs][Mixed ID]\n[Mixed ID][]\n[Mixed ID]\n![Photo][mixed id]\n\n[mixed id]: <https://example.org/reference> "Title"';
  for (const label of ['Docs', '[Mixed ID][]', '[Mixed ID]\n!', 'Photo', 'example.org/reference']) {
    expect(link(source, label)).toBe('https://example.org/reference');
  }
  expect(link('[Missing][unknown]', 'Missing')).toBeNull();
});

test('recognizes bare URLs and excludes sentence punctuation without confusing adjacent text', () => {
  expect(link('See https://example.org/a_(b), then https://example.org/two.', 'a_(b)')).toBe('https://example.org/a_(b)');
  expect(link('See https://example.org/a_(b), then https://example.org/two.', '/two')).toBe('https://example.org/two');
  expect(link('Other text https://example.org', 'Other')).toBeNull();
});

test('does not open code, local images, relative notes or non-web protocols', () => {
  for (const source of ['`https://example.org`', '```md\n[Docs](https://example.org)\n```', '    https://example.org',
    '[Docs](javascript:alert(1))', '[Docs](file:///C:/notes/a.md)', '[Docs](mailto:user@example.org)',
    '[Docs](../a.md)', '![Docs](lownotes-image:abc.webp)']) {
    expect(link(source, source.includes('Docs') ? 'Docs' : 'example.org')).toBeNull();
  }
});

test('resolves links beyond the initially parsed viewport and after document edits', () => {
  let state = EditorState.create({ doc: 'Text\n'.repeat(1500) + '[Docs](https://example.org/old)', extensions: [markdown()] });
  const position = state.doc.length - 28;
  expect(editorLinkAt(state, position)).toBe('https://example.org/old');
  state = state.update({ changes: { from: state.doc.length - 4, to: state.doc.length - 1, insert: 'new' } }).state;
  expect(editorLinkAt(state, position)).toBe('https://example.org/new');
  expect(editorLinkAt(state, state.doc.length)).toBeNull();
});
