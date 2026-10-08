import { describe, expect, test } from 'bun:test';
import type { VaultItem } from '../src/lib/types';
import { visibleNoteRows } from '../src/lib/note-tree';
import { resolveNoteLink } from '../src/lib/note-links';
import { connectDraftCollection, groupDraftPaths } from '../src/lib/draft-paths';

const item = (path: string, is_dir = false): VaultItem => ({
  path, is_dir, name: path.split('/').at(-1)!, title: path.split('/').at(-1)!.replace(/\.md$/, ''),
  modified_ms: 0, size: 0,
});

const items = [item('Python', true), item('Python/Plano.md'), item('Python/Etapas', true),
  item('Python/Etapas/Fundamentos.md'), item('Outro', true), item('Outro/Plano.md')];

describe('hierarquia e links de notas', () => {
  test('mostra pastas aninhadas, recolhe e revela ancestrais na busca', () => {
    expect(visibleNoteRows(items, new Set()).map((row) => [row.item.path, row.depth])).toEqual([
      ['Outro', 0], ['Outro/Plano.md', 1], ['Python', 0],
      ['Python/Etapas', 1], ['Python/Etapas/Fundamentos.md', 2], ['Python/Plano.md', 1],
    ]);
    expect(visibleNoteRows(items, new Set(['Python'])).map((row) => row.item.path))
      .toEqual(['Outro', 'Outro/Plano.md', 'Python']);
    expect(visibleNoteRows(items, new Set(['Python']), 'Fundamentos').map((row) => row.item.path))
      .toEqual(['Python', 'Python/Etapas', 'Python/Etapas/Fundamentos.md']);
  });

  test('resolve primeiro caminho relativo e não escolhe títulos ambíguos', () => {
    expect(resolveNoteLink(items, 'Python/Etapas/Fundamentos.md', '../Plano.md#Objetivos'))
      .toBe('Python/Plano.md');
    expect(resolveNoteLink(items, 'Python/Etapas/Fundamentos.md', 'Python/Plano|Plano'))
      .toBe('Python/Plano.md');
    expect(resolveNoteLink(items, '', 'Plano')).toBeNull();
  });

  test('links Markdown preservam a base relativa e wikilinks preferem o caminho do vault', () => {
    const notes = [item('folder/source.md'), item('folder/sub/target.md'), item('sub/target.md')];
    expect(resolveNoteLink(notes, 'folder/source.md', 'sub/target.md#etapa', 'markdown')).toBe('folder/sub/target.md');
    expect(resolveNoteLink(notes, 'folder/source.md', 'sub/target#etapa', 'wiki')).toBe('sub/target.md');
    expect(resolveNoteLink(notes, 'folder/source.md', '/sub/target.md', 'markdown')).toBe('sub/target.md');
  });

  test('destinos codificados após renomear abrem sem tratar uma URL externa como nota', () => {
    const notes = [item('folder/ação (nova).md')];
    expect(resolveNoteLink(notes, 'folder/source.md', 'a%C3%A7%C3%A3o%20%28nova%29.md?view=1#seção', 'markdown')).toBe(notes[0].path);
    expect(resolveNoteLink(notes, '', 'https://example.org/note.md', 'markdown')).toBeNull();
    expect(resolveNoteLink(notes, '', 'bad%XX.md', 'markdown')).toBeNull();
  });

  test('agrupa uma série gerada sem mover caminhos já definidos', () => {
    expect(groupDraftPaths('Crie notas para aprender Python', [
      { path: 'Plano.md', content: 'a' }, { path: 'Etapas.md', content: 'b' },
    ]).map((draft) => draft.path)).toEqual(['Python/Plano.md', 'Python/Etapas.md']);
    expect(groupDraftPaths('Python', [
      { path: 'Python/Plano.md', content: 'a' }, { path: 'Etapas.md', content: 'b' },
    ]).map((draft) => draft.path)).toEqual(['Python/Plano.md', 'Python/Etapas.md']);
  });

  test('liga o plano aos demais rascunhos da coleção', () => {
    const drafts = connectDraftCollection(groupDraftPaths('Aprender Python', [
      { path: 'Etapas.md', content: '# Etapas\n' },
      { path: 'Plano.md', content: '# Plano\n' },
      { path: 'Checklist.md', content: '# Checklist\n' },
    ]));
    expect(drafts[1].content).toContain('[[Python/Etapas|Etapas]]');
    expect(drafts[1].content).toContain('[[Python/Checklist|Checklist]]');
    expect(drafts[0].content).toBe('# Etapas\n');
  });
});
