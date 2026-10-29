import { markdown } from './markdown';

/** Fence language of drawings kept inside notes, as the Obsidian plugin calls them. */
export const DRAWING_LANGUAGE = 'excalidraw';

/** Deleted elements stay this long so editors open on other devices learn about the deletion. */
const TOMBSTONE_MS = 24 * 60 * 60 * 1000;

export interface DrawingElement { id: string; type: string; version?: number; isDeleted?: boolean; updated?: number; fileId?: string | null; [key: string]: unknown }
export interface DrawingScene {
  elements: DrawingElement[];
  appState: Record<string, unknown>;
  files: Record<string, unknown>;
}

/** A drawing fence in the note: `from`..`to` covers the lines between its fences. */
export interface DrawingBlock { index: number; line: number; from: number; to: number; indent: string; closed: boolean; content: string }

export function isDrawingFence(info: string): boolean {
  return info.trim().split(/\s+/, 1)[0] === DRAWING_LANGUAGE;
}

export function findDrawingBlocks(text: string): DrawingBlock[] {
  const starts = [0];
  for (let i = text.indexOf('\n'); i >= 0; i = text.indexOf('\n', i + 1)) starts.push(i + 1);
  const lineStart = (line: number) => line < starts.length ? starts[line] : text.length;
  const blocks: DrawingBlock[] = [];
  for (const token of markdown.parse(text, {})) {
    if (token.type !== 'fence' || !token.map || !isDrawingFence(token.info)) continue;
    const [open, end] = token.map;
    const opening = text.slice(lineStart(open), lineStart(open + 1));
    const closing = text.slice(lineStart(end - 1), lineStart(end));
    const closed = end - 1 > open && /^[ \t>]*(`{3,}|~{3,})\s*$/.test(closing.replace(/\r?\n$/, ''));
    // Quote markers stay; a list marker becomes the spaces that continue the item.
    const prefix = /^([ \t>]*)((?:[-*+]|\d+[.)])[ \t]+)?/.exec(opening)!;
    blocks.push({ index: blocks.length, line: open + 1, from: lineStart(open + 1), to: lineStart(closed ? end - 1 : end),
      indent: prefix[1] + ' '.repeat(prefix[2]?.length ?? 0), closed, content: token.content });
  }
  return blocks;
}

/** The smallest text change that turns the block's lines into `content`.
 * Small changes let concurrent edits of different elements merge in the CRDT. */
export function drawingChange(text: string, block: DrawingBlock, content: string): { from: number; to: number; insert: string } | null {
  const newline = text.includes('\r\n') ? '\r\n' : '\n';
  let body = content.split('\n').filter((line, i, lines) => i < lines.length - 1 || line).map(line => `${block.indent}${line}${newline}`).join('');
  if (!block.closed) body += `${block.indent}\`\`\`${newline}`;
  const before = text.slice(block.from, block.to);
  if (before === body) return null;
  let start = 0;
  while (start < before.length && start < body.length && before[start] === body[start]) start++;
  let end = 0;
  while (end < before.length - start && end < body.length - start && before[before.length - 1 - end] === body[body.length - 1 - end]) end++;
  return { from: block.from + start, to: block.to - end, insert: body.slice(start, body.length - end) };
}

/** A new drawing block for the editor at `position`, on lines of its own. */
export function drawingInsertion(text: string, position: number): { from: number; insert: string; contentOffset: number } {
  const newline = text.includes('\r\n') ? '\r\n' : '\n';
  const lineStart = text.lastIndexOf('\n', position - 1) + 1;
  const atLineStart = text.slice(lineStart, position).trim() === '';
  const from = atLineStart ? lineStart : position;
  const before = from === 0 || text.slice(0, from).endsWith(`${newline}${newline}`) ? '' : atLineStart ? newline : `${newline}${newline}`;
  const opening = `${before}\`\`\`${DRAWING_LANGUAGE}${newline}`;
  const insert = `${opening}${serializeDrawing(emptyDrawing()).replaceAll('\n', newline)}\`\`\`${newline}${text.slice(from).startsWith(newline) || from === text.length ? '' : newline}`;
  return { from, insert, contentOffset: from + opening.length };
}

export function emptyDrawing(): DrawingScene {
  return { elements: [], appState: { viewBackgroundColor: '#ffffff' }, files: {} };
}

function isElement(value: unknown): value is DrawingElement {
  return !!value && typeof value === 'object' && typeof (value as DrawingElement).id === 'string' && typeof (value as DrawingElement).type === 'string';
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

/** Reads a drawing block. Content damaged by a merge keeps every element line that still parses. */
export function parseDrawing(content: string): DrawingScene | null {
  if (!content.trim()) return emptyDrawing();
  try {
    const data = JSON.parse(content);
    if (!data || typeof data !== 'object' || !Array.isArray(data.elements)) return null;
    return { elements: data.elements.filter(isElement), appState: { ...emptyDrawing().appState, ...record(data.appState) }, files: record(data.files) };
  } catch {
    const scene = emptyDrawing();
    const elements = new Map<string, DrawingElement>();
    let section = '';
    for (const raw of content.split(/\r?\n/)) {
      const line = raw.trim().replace(/,$/, '');
      const key = /^"(elements|files|appState)"\s*:/.exec(line)?.[1];
      if (key) section = key;
      if (key === 'appState') { try { scene.appState = { ...scene.appState, ...record(JSON.parse(line.slice(line.indexOf(':') + 1))) }; } catch { /* keep defaults */ } continue; }
      if (!line.startsWith('{') && !line.startsWith('"')) continue;
      try {
        if (section === 'elements') {
          const element = JSON.parse(line);
          const known = elements.get(element?.id);
          if (isElement(element) && (!known || (element.version ?? 0) >= (known.version ?? 0))) elements.set(element.id, element);
        } else if (section === 'files') Object.assign(scene.files, JSON.parse(`{${line}}`));
      } catch { /* a damaged line is dropped */ }
    }
    if (elements.size === 0) return null;
    scene.elements = [...elements.values()];
    return scene;
  }
}

/** One element per line, so CRDT merges of different elements keep valid JSON. */
export function serializeDrawing(scene: DrawingScene, now = Date.now()): string {
  const elements = scene.elements.filter(element => !element.isDeleted || now - (element.updated ?? 0) < TOMBSTONE_MS);
  const used = new Set(elements.filter(element => !element.isDeleted && element.fileId).map(element => element.fileId));
  const files = Object.entries(scene.files).filter(([id]) => used.has(id));
  const list = (entries: string[]) => entries.map((entry, i) => `${entry}${i < entries.length - 1 ? ',' : ''}\n`).join('');
  return '{\n"type": "excalidraw",\n"version": 2,\n"source": "lownotes",\n'
    + `"elements": [\n${list(elements.map(element => JSON.stringify(element)))}],\n`
    + `"appState": ${JSON.stringify(scene.appState)},\n`
    + `"files": {\n${list(files.map(([id, file]) => `${JSON.stringify(id)}: ${JSON.stringify(file)}`))}}\n}\n`;
}
