import type { VaultItem } from './types';

/** Resolve a link from the current note, preferring an exact path over a title. */
export function resolveNoteLink(items: VaultItem[], source: string, token: string, kind: 'wiki' | 'markdown' = 'wiki'): string | null {
  let clean: string;
  try {
    clean = decodeURIComponent(token.split('|', 1)[0].split(/[?#]/, 1)[0].trim()
      .replace(/^<|>$/g, '').replaceAll('\\', '/'));
  } catch { return null; }
  if (!clean || clean.includes(':') || clean.startsWith('//')) return null;
  const parent = source.includes('/') ? source.slice(0, source.lastIndexOf('/')) : '';
  const normalize = (path: string): string | null => {
    const parts: string[] = [];
    for (const part of path.split('/')) {
      if (!part || part === '.') continue;
      if (part === '..') { if (!parts.length) return null; parts.pop(); }
      else parts.push(part);
    }
    return parts.join('/');
  };
  const candidates = clean.startsWith('/') ? [clean.slice(1)] : clean.startsWith('./') || clean.startsWith('../')
    ? [`${parent}/${clean}`]
    : kind === 'wiki' && clean.includes('/') ? [clean, `${parent}/${clean}`] : [`${parent}/${clean}`, clean];
  for (const candidate of candidates) {
    const path = normalize(candidate)?.toLocaleLowerCase();
    if (!path) continue;
    const full = /\.(?:md|markdown)$/.test(path) ? path : `${path}.md`;
    const match = items.find((item) => !item.is_dir &&
      (item.path.toLocaleLowerCase() === path || item.path.toLocaleLowerCase() === full));
    if (match) return match.path;
  }
  if (clean.includes('/')) return null;
  const title = clean.replace(/\.(?:md|markdown)$/i, '').toLocaleLowerCase();
  const matches = items.filter((item) => !item.is_dir && item.title.toLocaleLowerCase() === title);
  return matches.length === 1 ? matches[0].path : null;
}
