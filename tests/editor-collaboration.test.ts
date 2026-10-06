import { expect, test } from 'bun:test';
import { taskCheckboxChange } from '../src/lib/markdown-tasks';
import { basicSetup, EditorView } from 'codemirror';
import { EditorState } from '@codemirror/state';
import { keymap } from '@codemirror/view';
import { ySyncAnnotation, ySyncFacet } from 'y-codemirror.next';
import * as Y from 'yjs';
import { createLocalCollaboration } from '../src/lib/editor-collaboration';

function peer(snapshot?: Uint8Array) {
  const doc = new Y.Doc();
  if (snapshot) Y.applyUpdate(doc, snapshot, 'init');
  const text = doc.getText('content');
  const collaboration = createLocalCollaboration(text, null);
  let state = EditorState.create({
    doc: text.toString(), extensions: [basicSetup, collaboration.extension],
  });
  const origin = state.facet(ySyncFacet);
  // A headless editor state lets us exercise the actual configured shortcut order.
  text.observe(() => {
    state = state.update({
      changes: { from: 0, to: state.doc.length, insert: text.toString() },
      annotations: ySyncAnnotation.of(origin),
    }).state;
  });
  return {
    doc, text, undoManager: collaboration.undoManager,
    edit(change: () => void) { doc.transact(change, origin); },
    shortcut(key: string) {
      const binding = state.facet(keymap).flat().find((binding) => binding.key === key);
      if (!binding?.run) throw new Error(`Missing shortcut: ${key}`);
      return binding.run({
        state,
        dispatch(transaction: Parameters<EditorState['update']>[0]) {
          state = state.update(transaction).state;
        },
      } as unknown as EditorView);
    },
    content: () => state.doc.toString(),
    destroy() { collaboration.undoManager.destroy(); doc.destroy(); },
  };
}

function connect(a: ReturnType<typeof peer>, b: ReturnType<typeof peer>) {
  a.doc.on('update', (update: Uint8Array, origin: unknown) => {
    if (origin !== 'remote') Y.applyUpdate(b.doc, update, 'remote');
  });
  b.doc.on('update', (update: Uint8Array, origin: unknown) => {
    if (origin !== 'remote') Y.applyUpdate(a.doc, update, 'remote');
  });
}

test('preview task edits synchronize and undo independently from remote typing', () => {
  const seed = new Y.Doc();
  seed.getText('content').insert(0, '🙂\n- [ ] Repetida\n- [ ] Repetida\n');
  const a = peer(Y.encodeStateAsUpdate(seed));
  const b = peer(Y.encodeStateAsUpdate(seed));
  connect(a, b);
  try {
    const offset = a.content().lastIndexOf('[ ]') + 1;
    const change = taskCheckboxChange(a.content(), offset, false, true)!;
    a.undoManager.stopCapturing();
    a.edit(() => { a.text.delete(change.from, 1); a.text.insert(change.from, change.insert); });
    a.undoManager.stopCapturing();
    b.edit(() => b.text.insert(b.text.length, 'Texto remoto.'));
    expect(a.content()).toBe('🙂\n- [ ] Repetida\n- [x] Repetida\nTexto remoto.');
    expect(b.content()).toBe(a.content());
    a.shortcut('Mod-z');
    expect(a.content()).toBe('🙂\n- [ ] Repetida\n- [ ] Repetida\nTexto remoto.');
    a.shortcut('Mod-y');
    expect(b.content()).toBe(a.content());
    expect(a.content()).toContain('- [x] Repetida');
  } finally { a.destroy(); b.destroy(); seed.destroy(); }
});

test('Ctrl+Z never undoes initial content or remote-only typing', () => {
  const seed = new Y.Doc();
  seed.getText('content').insert(0, 'Initial.');
  const a = peer(Y.encodeStateAsUpdate(seed));
  const b = peer(Y.encodeStateAsUpdate(seed));
  connect(a, b);
  try {
    b.edit(() => b.text.insert(b.text.length, ' Remote.'));
    expect(a.undoManager.undoStack).toHaveLength(0);
    expect(a.shortcut('Mod-z')).toBeTruthy();
    expect(a.content()).toBe('Initial. Remote.');
    expect(b.content()).toBe(a.content());
  } finally { a.destroy(); b.destroy(); seed.destroy(); }
});

test('undo and redo affect only the local device with interleaved remote text', () => {
  const a = peer();
  const b = peer(Y.encodeStateAsUpdate(a.doc));
  connect(a, b);
  try {
    a.edit(() => a.text.insert(0, 'Local.'));
    b.edit(() => b.text.insert(b.text.length, ' Remote.'));
    a.undoManager.stopCapturing();
    a.edit(() => a.text.insert(a.text.length, ' More.'));
    a.shortcut('Mod-z');
    expect(a.content()).toBe('Local. Remote.');
    a.shortcut('Mod-z');
    expect(a.content()).toBe(' Remote.');
    a.shortcut('Mod-z');
    expect(a.content()).toBe(' Remote.');
    a.shortcut('Mod-y');
    expect(a.content()).toBe('Local. Remote.');
    a.shortcut('Mod-Shift-z');
    expect(a.content()).toBe('Local. Remote. More.');
    b.shortcut('Mod-z');
    expect(a.content()).toBe('Local. More.');
    expect(b.content()).toBe(a.content());
  } finally { a.destroy(); b.destroy(); }
});

test('remote edits received after undo preserve the local redo stack', () => {
  const a = peer();
  const b = peer(Y.encodeStateAsUpdate(a.doc));
  connect(a, b);
  try {
    a.edit(() => a.text.insert(0, 'Local.'));
    a.shortcut('Mod-z');
    b.edit(() => b.text.insert(0, 'Remote.'));
    a.shortcut('Mod-y');
    expect(a.content()).toContain('Local.');
    expect(a.content()).toContain('Remote.');
    expect(b.content()).toBe(a.content());
  } finally { a.destroy(); b.destroy(); }
});

test('undoing a local deletion preserves text inserted by the other device', () => {
  const a = peer();
  a.text.insert(0, 'Original.');
  const b = peer(Y.encodeStateAsUpdate(a.doc));
  connect(a, b);
  try {
    a.edit(() => a.text.delete(0, a.text.length));
    b.edit(() => b.text.insert(0, 'Remote.'));
    a.shortcut('Mod-z');
    expect(a.content()).toContain('Original.');
    expect(a.content()).toContain('Remote.');
    expect(b.content()).toBe(a.content());
  } finally { a.destroy(); b.destroy(); }
});
