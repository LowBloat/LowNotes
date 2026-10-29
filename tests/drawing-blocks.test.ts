import { expect, test } from 'bun:test';
import { drawingChange, drawingInsertion, emptyDrawing, findDrawingBlocks, parseDrawing, serializeDrawing, type DrawingElement } from '../src/lib/drawing-blocks';

const element = (id: string, extra: Partial<DrawingElement> = {}): DrawingElement => ({ id, type: 'rectangle', version: 1, x: 0, y: 0, ...extra });
const apply = (text: string, change: { from: number; to: number; insert: string } | null) =>
  change ? text.slice(0, change.from) + change.insert + text.slice(change.to) : text;

test('drawing blocks are found in order with their content lines, also inside lists and quotes', () => {
  const source = '# A\n\n```excalidraw\n{"elements":[]}\n```\n\n```js\nx\n```\n\n- item\n  ```excalidraw\n  {"a":1}\n  ```\n\n> ```excalidraw\n> {"b":2}\n> ```\n';
  const blocks = findDrawingBlocks(source);
  expect(blocks.map(block => [block.index, block.line, block.indent, block.closed, block.content])).toEqual([
    [0, 3, '', true, '{"elements":[]}\n'],
    [1, 12, '  ', true, '{"a":1}\n'],
    [2, 16, '> ', true, '{"b":2}\n'],
  ]);
  expect(source.slice(blocks[1].from, blocks[1].to)).toBe('  {"a":1}\n');
  const next = apply(source, drawingChange(source, blocks[1], '{"a":2}\n'));
  expect(findDrawingBlocks(next)[1].content).toBe('{"a":2}\n');
  expect(next.replace('{"a":2}', '{"a":1}')).toBe(source);
  expect(drawingChange(source, blocks[0], '{"elements":[]}\n')).toBeNull();
});

test('an unclosed drawing fence is closed when it is saved, and CRLF notes keep CRLF', () => {
  const source = 'Intro\r\n\r\n```excalidraw\r\n{"x":1}\r\n';
  const [block] = findDrawingBlocks(source);
  expect(block.closed).toBe(false);
  const next = apply(source, drawingChange(source, block, '{"x":2}\n'));
  expect(next).toBe('Intro\r\n\r\n```excalidraw\r\n{"x":2}\r\n```\r\n');
  expect(findDrawingBlocks(next)[0]).toMatchObject({ closed: true, content: '{"x":2}\n' });
});

test('a new drawing is inserted on lines of its own and starts empty', () => {
  for (const [text, position] of [['', 0], ['hello', 5], ['hello', 2], ['a\n\nb', 2], ['a\r\nb', 1]] as const) {
    const insertion = drawingInsertion(text, position);
    const next = text.slice(0, insertion.from) + insertion.insert + text.slice(insertion.from);
    const blocks = findDrawingBlocks(next);
    expect(blocks).toHaveLength(1);
    expect(blocks[0].from).toBe(insertion.contentOffset);
    expect(parseDrawing(blocks[0].content)).toEqual(emptyDrawing());
    expect(next.replace(/```excalidraw[\s\S]*?```\r?\n/, '').replace(/\s/g, '')).toBe(text.replace(/\s/g, ''));
  }
});

test('drawings serialize one element per line and parse back', () => {
  const scene = { elements: [element('a'), element('b', { fileId: 'f1', type: 'image' })], appState: { viewBackgroundColor: '#fff' }, files: { f1: { id: 'f1' }, unused: { id: 'unused' } } };
  const text = serializeDrawing(scene);
  expect(text.split('\n').filter(line => line.startsWith('{"id"'))).toHaveLength(2);
  expect(parseDrawing(text)).toEqual({ ...scene, files: { f1: { id: 'f1' } } });
  expect(parseDrawing('')).toEqual(emptyDrawing());
  expect(parseDrawing('not json')).toBeNull();
});

test('old deleted elements are dropped while recent deletions stay for other open editors', () => {
  const now = 10 * 24 * 60 * 60 * 1000;
  const text = serializeDrawing({ elements: [element('old', { isDeleted: true, updated: 0 }), element('recent', { isDeleted: true, updated: now - 1000 }), element('kept')], appState: {}, files: {} }, now);
  expect(parseDrawing(text)!.elements.map(item => item.id)).toEqual(['recent', 'kept']);
});

test('concurrent appends merge into valid JSON and damaged lines are skipped', () => {
  const base = serializeDrawing({ elements: [element('a')], appState: {}, files: {} });
  const [block] = findDrawingBlocks(`\`\`\`excalidraw\n${base}\`\`\`\n`);
  const source = `\`\`\`excalidraw\n${base}\`\`\`\n`;
  const first = drawingChange(source, block, serializeDrawing({ elements: [element('a'), element('b')], appState: {}, files: {} }))!;
  const second = drawingChange(source, block, serializeDrawing({ elements: [element('a'), element('c')], appState: {}, files: {} }))!;
  expect(first.from).toBe(second.from);
  const merged = source.slice(0, first.from) + first.insert + second.insert + source.slice(first.to);
  expect(parseDrawing(findDrawingBlocks(merged)[0].content)!.elements.map(item => item.id)).toEqual(['a', 'b', 'c']);
  const damaged = base.replace('{"id":"a"', '{"id":"a"BROKEN').replace('"elements": [\n', `"elements": [\n${JSON.stringify(element('d', { version: 3 }))},\n${JSON.stringify(element('d', { version: 2 }))},\n`);
  expect(parseDrawing(damaged)!.elements).toEqual([element('d', { version: 3 })]);
});
