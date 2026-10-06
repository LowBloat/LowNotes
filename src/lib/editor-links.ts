import type { EditorState } from '@codemirror/state';
import { ensureSyntaxTree, syntaxTree } from '@codemirror/language';
import { EditorView } from '@codemirror/view';
import type { SyntaxNode } from '@lezer/common';
import { markdown } from './markdown';

const references = new WeakMap<EditorState, Record<string, { href: string; title: string }>>();

function externalUrl(value: string): string | null {
  const destination = markdown.utils.unescapeAll(value.replace(/^<([\s\S]*)>$/, '$1'));
  if (!/^https?:\/\//i.test(destination) || /[\u0000-\u0020]/.test(destination)) return null;
  try {
    const url = new URL(destination);
    return url.protocol === 'https:' || url.protocol === 'http:' ? url.href : null;
  } catch { return null; }
}

/** Resolve the link under the pointer, without treating code or local image IDs as URLs. */
export function editorLinkAt(state: EditorState, position: number): string | null {
  if (position < 0 || position >= state.doc.length) return null;
  const tree = ensureSyntaxTree(state, position + 1, 50) ?? syntaxTree(state);
  for (let node: SyntaxNode | null = tree.resolveInner(position, 1); node; node = node.parent) {
    if (['FencedCode', 'CodeBlock', 'InlineCode', 'HTMLBlock', 'HTMLTag'].includes(node.name)) return null;
    if (['Link', 'Image', 'Autolink', 'LinkReference'].includes(node.name)) {
      const url = node.getChild('URL');
      if (url) {
        // In reference definitions only the URL itself is clickable.
        if (node.name === 'LinkReference' && (position < url.from || position >= url.to)) return null;
        return externalUrl(state.sliceDoc(url.from, url.to));
      }
      const label = node.getChild('LinkLabel');
      const close = node.getChildren('LinkMark').find(mark => state.sliceDoc(mark.from, mark.to) === ']');
      if (!close) return null;
      const explicit = label && state.sliceDoc(label.from + 1, label.to - 1);
      const reference = markdown.utils.normalizeReference(explicit || state.sliceDoc(node.from + (node.name === 'Image' ? 2 : 1), close.from));
      let definitions = references.get(state);
      if (!definitions) {
        const env: { references?: Record<string, { href: string; title: string }> } = {};
        markdown.parse(state.doc.toString(), env);
        definitions = env.references ?? {};
        references.set(state, definitions);
      }
      return externalUrl(definitions[reference]?.href ?? '');
    }
  }
  const line = state.doc.lineAt(position);
  const matches = markdown.linkify.match(line.text) ?? [];
  const match = matches.find(link => position >= line.from + link.index && position < line.from + link.lastIndex);
  return match ? externalUrl(match.url) : null;
}

export function openEditorLinks(open: (url: string) => Promise<unknown>, onError: () => void) {
  return EditorView.domEventHandlers({
    mousedown(event, view) {
      if (event.button !== 0 || !(event.ctrlKey || event.metaKey) || event.altKey) return false;
      const position = view.posAtCoords({ x: event.clientX, y: event.clientY }, false);
      const url = position === null ? null : editorLinkAt(view.state, position);
      if (!url) return false;
      event.preventDefault();
      void open(url).catch(onError);
      return true;
    },
    mousemove(event, view) {
      const position = event.ctrlKey || event.metaKey ? view.posAtCoords({ x: event.clientX, y: event.clientY }, false) : null;
      view.contentDOM.style.cursor = position !== null && editorLinkAt(view.state, position) ? 'pointer' : '';
      return false;
    },
    keyup(event, view) {
      if (!event.ctrlKey && !event.metaKey) view.contentDOM.style.cursor = '';
      return false;
    },
    mouseleave(_event, view) { view.contentDOM.style.cursor = ''; return false; },
    blur(_event, view) { view.contentDOM.style.cursor = ''; return false; },
  });
}
