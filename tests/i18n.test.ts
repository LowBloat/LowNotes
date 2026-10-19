import { expect, test } from 'bun:test';
import enUS from '../src/lib/i18n/locales/en-US';
import ptBR from '../src/lib/i18n/locales/pt-BR';
import esES from '../src/lib/i18n/locales/es-ES';
import { resolveLocale, translate } from '../src/lib/i18n';

function flatten(obj: Record<string, unknown>, prefix = ''): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [key, value] of Object.entries(obj)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (typeof value === 'string') {
      out[path] = value;
    } else {
      Object.assign(out, flatten(value as Record<string, unknown>, path));
    }
  }
  return out;
}

function placeholders(value: string): string[] {
  return [...value.matchAll(/\{([a-zA-Z0-9_]+)\}/g)].map((m) => m[1]).sort();
}

test('todos os locales compartilham o mesmo conjunto de chaves', () => {
  const en = flatten(enUS as Record<string, unknown>);
  const pt = flatten(ptBR as Record<string, unknown>);
  const es = flatten(esES as Record<string, unknown>);

  expect(Object.keys(pt).sort()).toEqual(Object.keys(en).sort());
  expect(Object.keys(es).sort()).toEqual(Object.keys(en).sort());

  for (const [key, value] of Object.entries(en)) {
    expect(value.length).toBeGreaterThan(0);
    expect((pt[key] ?? '').length).toBeGreaterThan(0);
    expect((es[key] ?? '').length).toBeGreaterThan(0);
  }
});

test('placeholders de parametros coincidem entre locales', () => {
  const en = flatten(enUS as Record<string, unknown>);
  const pt = flatten(ptBR as Record<string, unknown>);
  const es = flatten(esES as Record<string, unknown>);

  for (const [key, value] of Object.entries(en)) {
    expect(placeholders(pt[key] ?? '')).toEqual(placeholders(value));
    expect(placeholders(es[key] ?? '')).toEqual(placeholders(value));
  }
});

test('resolveLocale respeita idioma salvo e detecta navegador', () => {
  expect(resolveLocale('es-ES')).toBe('es-ES');
  expect(resolveLocale('pt-BR', 'en-US')).toBe('pt-BR');
  expect(resolveLocale('', 'pt-BR')).toBe('pt-BR');
  expect(resolveLocale(undefined, 'es-ES')).toBe('es-ES');
  expect(resolveLocale(null, 'pt')).toBe('pt-BR');
  expect(resolveLocale('', 'fr-FR')).toBe('en-US');
});

test('translate interpola parametros e cai para en-US', () => {
  expect(translate('pt-BR', 'sidebar.pairedMany', { count: 3 })).toBe('3 pareados');
  expect(translate('en-US', 'editor.words', { count: 10 })).toBe('10 words');
  expect(translate('pt-BR', 'errors.noteExists')).toBe('uma nota com este nome já existe');
  expect(translate('pt-BR', 'chave.inexistente')).toBe('chave.inexistente');
});

test('counts pick singular and plural forms, with zero using the plural', () => {
  expect(translate('en-US', 'editor.words', { count: 1 })).toBe('1 word');
  expect(translate('en-US', 'editor.words', { count: 2 })).toBe('2 words');
  expect(translate('en-US', 'editor.words', { count: 0 })).toBe('0 words');
  expect(translate('pt-BR', 'editor.words', { count: 1 })).toBe('1 palavra');
  expect(translate('pt-BR', 'editor.words', { count: 0 })).toBe('0 palavras');
  expect(translate('es-ES', 'ai.drafts', { count: 1 })).toBe('1 documento listo');
  expect(translate('es-ES', 'ai.drafts', { count: 3 })).toBe('3 documentos listos');
  // Keys without plural variants keep working.
  expect(translate('en-US', 'sidebar.pairedMany', { count: 1 })).toBe('1 paired devices');
});
