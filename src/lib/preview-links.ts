/** What a click on a link inside the rendered preview should do. The preview
 * never lets the WebView navigate on its own: that would replace the app. */
export type PreviewLinkAction =
  | { kind: 'note'; target: string; mode: 'wiki' | 'markdown' }
  | { kind: 'external'; url: string }
  | { kind: 'anchor'; id: string }
  | { kind: 'ignore' };

const EXTERNAL_PROTOCOLS = new Set(['http:', 'https:', 'mailto:']);

export function previewLinkAction(href: string | null, wikilink: string | null): PreviewLinkAction {
  if (wikilink !== null) return { kind: 'note', target: wikilink, mode: 'wiki' };
  const value = (href ?? '').trim();
  if (!value) return { kind: 'ignore' };
  if (value.startsWith('#')) {
    let id = value.slice(1);
    try { id = decodeURIComponent(id); } catch { /* keep the raw fragment */ }
    return id ? { kind: 'anchor', id } : { kind: 'ignore' };
  }
  const hasScheme = /^[a-z][\w+.-]*:/i.test(value);
  if (!hasScheme && /\.(?:md|markdown)(?:[?#].*)?$/i.test(value)) return { kind: 'note', target: value, mode: 'markdown' };
  if (!hasScheme || /[\u0000-\u0020]/.test(value)) return { kind: 'ignore' };
  try {
    const url = new URL(value);
    if (!EXTERNAL_PROTOCOLS.has(url.protocol)) return { kind: 'ignore' };
    if (url.protocol !== 'mailto:' && !url.hostname) return { kind: 'ignore' };
    return { kind: 'external', url: value };
  } catch {
    return { kind: 'ignore' };
  }
}
