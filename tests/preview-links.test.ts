import { expect, test } from 'bun:test';
import { previewLinkAction } from '../src/lib/preview-links';

test('wikilinks and relative Markdown destinations open notes', () => {
  expect(previewLinkAction('ignored', 'Folder/Note')).toEqual({ kind: 'note', target: 'Folder/Note', mode: 'wiki' });
  expect(previewLinkAction('sub/a%C3%A7%C3%A3o.md#etapa', null)).toEqual({ kind: 'note', target: 'sub/a%C3%A7%C3%A3o.md#etapa', mode: 'markdown' });
  expect(previewLinkAction('../notes/readme.markdown', null)).toMatchObject({ kind: 'note', mode: 'markdown' });
});

test('web and mail links go to the system handler', () => {
  expect(previewLinkAction('https://example.org/file.md', null)).toEqual({ kind: 'external', url: 'https://example.org/file.md' });
  expect(previewLinkAction('http://example.org', null)).toEqual({ kind: 'external', url: 'http://example.org' });
  expect(previewLinkAction('mailto:user@example.org', null)).toEqual({ kind: 'external', url: 'mailto:user@example.org' });
});

test('in-note anchors scroll the preview, including footnotes and encoded headings', () => {
  expect(previewLinkAction('#fn1', null)).toEqual({ kind: 'anchor', id: 'fn1' });
  expect(previewLinkAction('#a%C3%A7%C3%A3o', null)).toEqual({ kind: 'anchor', id: 'ação' });
  expect(previewLinkAction('#', null)).toEqual({ kind: 'ignore' });
});

test('everything else is ignored instead of navigating the WebView', () => {
  for (const href of ['', null, './report.pdf', 'docs/', 'javascript:alert(1)', 'file:///C:/a.md', 'ftp://example.org/a',
    'tel:+5511999999999', 'data:text/html,hi', 'lownotes-image:abc.webp', 'https://', 'https://exa mple.org']) {
    expect(previewLinkAction(href, null)).toEqual({ kind: 'ignore' });
  }
});
