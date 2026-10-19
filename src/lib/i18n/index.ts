import { derived, get, writable } from 'svelte/store';
import enUS from './locales/en-US';
import ptBR from './locales/pt-BR';
import esES from './locales/es-ES';
import type { Dictionary } from './types';

export const SUPPORTED_LOCALES = ['en-US', 'pt-BR', 'es-ES'] as const;
export type LocaleCode = (typeof SUPPORTED_LOCALES)[number];

export const LOCALE_LABELS: Record<LocaleCode, string> = {
  'en-US': 'English',
  'pt-BR': 'Português (Brasil)',
  'es-ES': 'Español',
};

export const LOCALE_SHORT_LABELS: Record<LocaleCode, string> = {
  'en-US': 'EN',
  'pt-BR': 'PT',
  'es-ES': 'ES',
};

const dictionaries: Record<LocaleCode, Dictionary> = {
  'en-US': enUS,
  'pt-BR': ptBR,
  'es-ES': esES,
};

export type TranslateParams = Record<string, string | number>;

type LeafPaths<T, Prefix extends string = ''> = {
  [K in keyof T & string]: T[K] extends string ? `${Prefix}${K}` : LeafPaths<T[K], `${Prefix}${K}.`>;
}[keyof T & string];

export type TKey = LeafPaths<Dictionary>;

function lookup(dict: Dictionary, key: string): string | undefined {
  let node: unknown = dict;
  for (const part of key.split('.')) {
    if (typeof node !== 'object' || node === null || !(part in node)) return undefined;
    node = (node as Record<string, unknown>)[part];
  }
  return typeof node === 'string' ? node : undefined;
}

const pluralRules = new Map<LocaleCode, Intl.PluralRules>();

/** Plural variant key such as `editor.words_one`. Zero uses the general form
 * in every supported language ("0 palavras", not "0 palavra"). */
function pluralKey(localeCode: LocaleCode, key: string, count: number): string {
  let rules = pluralRules.get(localeCode);
  if (!rules) pluralRules.set(localeCode, rules = new Intl.PluralRules(localeCode));
  return `${key}_${count === 0 ? 'other' : rules.select(count)}`;
}

function lookupCounted(localeCode: LocaleCode, key: string, count: unknown): string | undefined {
  const dictionary = dictionaries[localeCode];
  const variant = typeof count === 'number' ? lookup(dictionary, pluralKey(localeCode, key, count)) : undefined;
  return variant ?? lookup(dictionary, key);
}

export function translate(
  localeCode: LocaleCode,
  key: TKey | string,
  params?: TranslateParams,
): string {
  let text = lookupCounted(localeCode, key, params?.count) ?? lookupCounted('en-US', key, params?.count) ?? key;
  if (params) {
    for (const [name, value] of Object.entries(params)) {
      text = text.replaceAll(`{${name}}`, String(value));
    }
  }
  return text;
}

export const locale = writable<LocaleCode>('en-US');

export const t = derived(locale, (current) => (key: TKey, params?: TranslateParams) =>
  translate(current, key, params)
);

/** Non-reactive translation for script contexts (alerts, confirms, timestamps). */
export function ts(key: TKey, params?: TranslateParams): string {
  return translate(get(locale), key, params);
}

/**
 * Translates backend-originated messages, which arrive as stable `errors.*`
 * keys. Unknown messages pass through unchanged.
 */
export function trError(message: string): string {
  return translate(get(locale), message);
}

export function isSupportedLocale(value: string): value is LocaleCode {
  return (SUPPORTED_LOCALES as readonly string[]).includes(value);
}

export function resolveLocale(saved: string | null | undefined, detected?: string): LocaleCode {
  if (saved && isSupportedLocale(saved)) return saved;
  const navigatorLanguage = detected ?? (typeof navigator !== 'undefined' ? navigator.language : '');
  const lower = navigatorLanguage.toLowerCase();
  const exact = SUPPORTED_LOCALES.find((l) => l.toLowerCase() === lower);
  if (exact) return exact;
  const base = lower.split('-')[0];
  const byBase = SUPPORTED_LOCALES.find((l) => l.split('-')[0].toLowerCase() === base);
  return byBase ?? 'en-US';
}
