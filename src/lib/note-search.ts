import { StateEffect, StateField, Text, type EditorState } from '@codemirror/state';
import { Decoration, EditorView, type DecorationSet } from '@codemirror/view';
import { SearchQuery } from '@codemirror/search';
import { ensureSyntaxTree, syntaxTree } from '@codemirror/language';
import type { SyntaxNode } from '@lezer/common';
import type { ViewMode } from './types';

export interface SearchMatch { from: number; to: number }
interface TextPart extends SearchMatch { node: TextNode; sourceFrom: number; sourceTo: number }
type TextNode = globalThis.Text;
export interface PreviewMatch extends SearchMatch {
  parts: { node: TextNode; from: number; to: number }[];
  sourceFrom: number;
  sourceTo: number;
  text: string;
}
export interface NoteSearchMatch extends SearchMatch { sourceIndex: number; previewIndex: number }

export function searchMatches(text: Text, query: SearchQuery): SearchMatch[] {
  if (!query.valid) return [];
  const matches: SearchMatch[] = [];
  const cursor = query.getCursor(text);
  for (let next = cursor.next(); !next.done; next = cursor.next()) {
    // Empty regex matches have no text to highlight or replace in the floating finder.
    if (next.value.to > next.value.from) matches.push({ from: next.value.from, to: next.value.to });
  }
  return matches;
}

export const setNoteSearchHighlights = StateEffect.define<{ matches: SearchMatch[]; active: number }>();
export const noteSearchHighlights = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, transaction) {
    value = value.map(transaction.changes);
    for (const effect of transaction.effects) {
      if (effect.is(setNoteSearchHighlights)) {
        value = Decoration.set(effect.value.matches.filter(match => match.to > match.from).map((match, index) =>
          Decoration.mark({ class: index === effect.value.active ? 'note-search-match note-search-current' : 'note-search-match' }).range(match.from, match.to)), true);
      }
    }
    return value;
  },
  provide: field => EditorView.decorations.from(field),
});

export function clearPreviewSearch(root: HTMLElement) {
  for (const mark of root.querySelectorAll('mark[data-note-search]')) mark.replaceWith(...mark.childNodes);
  root.normalize();
}

/** Index rendered text across inline formatting, separating distinct blocks. */
export function previewSearchMatches(root: HTMLElement, sourceLength: number, query: SearchQuery): PreviewMatch[] {
  if (!query.valid) return [];
  const parts: TextPart[] = [];
  let text = '', previousBlock: Element | null = null;
  const walker = root.ownerDocument.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const node = walker.currentNode as TextNode;
    const parent = node.parentElement;
    if (!parent || parent.closest('svg, .mermaid-svg, script, style, button, [aria-hidden="true"]')) continue;
    const mapped = parent.closest<HTMLElement>('[data-search-from]');
    const block = parent.closest('p, h1, h2, h3, h4, h5, h6, pre, li, td, th, dt, dd') ?? mapped;
    if (!block || !node.data) continue;
    if (block !== previousBlock && text) text += '\n';
    previousBlock = block;
    const from = text.length;
    text += node.data;
    parts.push({ node, from, to: text.length,
      sourceFrom: Number(mapped?.dataset.searchFrom ?? 0), sourceTo: Number(mapped?.dataset.searchTo ?? sourceLength) });
  }
  const results: PreviewMatch[] = [];
  for (const match of searchMatches(Text.of(text.split('\n')), query)) {
    const fragments = parts.filter(part => part.to > match.from && part.from < match.to);
    if (!fragments.length) continue;
    results.push({ ...match, text: text.slice(match.from, match.to),
      sourceFrom: fragments[0].sourceFrom, sourceTo: fragments.at(-1)!.sourceTo,
      parts: fragments.map(part => ({ node: part.node, from: Math.max(0, match.from - part.from), to: Math.min(part.to - part.from, match.to - part.from) })) });
  }
  return results;
}

/** A split result is counted once when the same occurrence exists in both panes. */
export function combineSearchMatches(state: EditorState, source: SearchMatch[], preview: PreviewMatch[], mode: ViewMode): NoteSearchMatch[] {
  if (mode === 'edit' || !preview.length) return mode === 'preview' ? [] : source.map((match, sourceIndex) => ({ ...match, sourceIndex, previewIndex: -1 }));
  const tree = ensureSyntaxTree(state, state.doc.length, 100) ?? syntaxTree(state);
  const visible = (position: number) => {
    for (let node: SyntaxNode | null = tree.resolveInner(position, 1); node; node = node.parent) {
      if (node.name === 'URL' && node.parent?.name !== 'Autolink') return false;
      if (['Image', 'LinkTitle', 'LinkLabel', 'LinkMark', 'CodeInfo', 'CodeMark', 'HeaderMark', 'EmphasisMark'].includes(node.name)) return false;
    }
    return true;
  };
  const used = new Set<number>();
  const mapped = preview.map(match => {
    const sourceIndex = source.findIndex((candidate, index) => !used.has(index)
      && candidate.from >= match.sourceFrom && candidate.to <= match.sourceTo
      && state.sliceDoc(candidate.from, candidate.to).normalize('NFKD') === match.text.normalize('NFKD')
      && visible(candidate.from) && visible(candidate.to - 1));
    if (sourceIndex >= 0) used.add(sourceIndex);
    return sourceIndex;
  });
  if (mode === 'preview') return preview.map((match, previewIndex) => ({
    ...(mapped[previewIndex] >= 0 ? source[mapped[previewIndex]] : { from: match.sourceFrom, to: match.sourceTo }),
    sourceIndex: mapped[previewIndex], previewIndex,
  }));
  const matches = source.map((match, sourceIndex) => ({ ...match, sourceIndex, previewIndex: mapped.indexOf(sourceIndex) }));
  if (mode === 'split') {
    preview.forEach((match, previewIndex) => {
      if (mapped[previewIndex] < 0) matches.push({ from: match.sourceFrom, to: match.sourceTo, sourceIndex: -1, previewIndex });
    });
    matches.sort((a, b) => a.from - b.from || a.to - b.to);
  }
  return matches;
}

/** Wrap text fragments separately so links, formatting and interactive tasks retain their DOM. */
export function highlightPreviewSearch(matches: PreviewMatch[], active: number) {
  const fragments = matches.flatMap((match, index) => match.parts.map(part => ({ ...part, index })));
  for (const part of fragments.reverse()) {
    part.node.splitText(part.to);
    const text = part.node.splitText(part.from);
    const mark = text.ownerDocument.createElement('mark');
    mark.dataset.noteSearch = String(part.index);
    mark.className = part.index === active ? 'note-search-match note-search-current' : 'note-search-match';
    text.replaceWith(mark);
    mark.append(text);
  }
}

export function previewSourceTarget(root: HTMLElement, position: number): HTMLElement | null {
  const blocks = [...root.querySelectorAll<HTMLElement>('[data-search-from]')];
  const containing = blocks.filter(block => Number(block.dataset.searchFrom) <= position && Number(block.dataset.searchTo) > position);
  return containing.sort((a, b) => (Number(a.dataset.searchTo) - Number(a.dataset.searchFrom)) - (Number(b.dataset.searchTo) - Number(b.dataset.searchFrom)))[0]
    ?? blocks.sort((a, b) => Math.abs(Number(a.dataset.searchFrom) - position) - Math.abs(Number(b.dataset.searchFrom) - position))[0] ?? null;
}
