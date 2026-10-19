import type { StateInline } from 'markdown-it';
import MarkdownIt from 'markdown-it';
import abbr from 'markdown-it-abbr';
import container from 'markdown-it-container';
import deflist from 'markdown-it-deflist';
import { full as emoji } from 'markdown-it-emoji';
import footnote from 'markdown-it-footnote';
import ins from 'markdown-it-ins';
import mark from 'markdown-it-mark';
import sub from 'markdown-it-sub';
import sup from 'markdown-it-sup';
import taskLists from 'markdown-it-task-lists';
import { interactiveTasks, type TaskPreviewOptions } from './markdown-tasks';
import { previewSourceMap } from './markdown-source-map';

export const markdown = new MarkdownIt({
  html: false,
  linkify: true,
  typographer: true,
})
  .use(abbr)
  .use(deflist)
  .use(emoji)
  .use(footnote)
  .use(ins)
  .use(mark)
  .use(sub)
  .use(sup)
  .use(taskLists, { enabled: false, label: true })
  .use(interactiveTasks);

for (const name of ['warning', 'info', 'tip', 'danger']) {
  markdown.use(container, name);
}

function wikilinkRule(state: StateInline, silent: boolean): boolean {
  const src = state.src;
  const start = state.pos;
  if (src.charCodeAt(start) !== 0x5b /* [ */ || src.charCodeAt(start + 1) !== 0x5b) return false;
  const close = src.indexOf(']]', start + 2);
  if (close < 0) return false;
  const content = src.slice(start + 2, close);
  if (content.length === 0 || content.includes('\n') || content.includes(']]')) return false;
  if (!silent) {
    const token = state.push('wikilink', '', 0);
    token.content = content;
  }
  state.pos = close + 2;
  return true;
}

markdown.inline.ruler.before('link', 'wikilink', wikilinkRule);
markdown.renderer.rules.wikilink = (tokens, index) => {
  const raw = tokens[index].content;
  const label = markdown.utils.escapeHtml(raw.split('|', 2)[1] || raw.split('|', 1)[0]);
  const target = markdown.utils.escapeHtml(raw.split('|', 1)[0]);
  return `<a href="#" data-wikilink="${target}" class="wikilink">${label}</a>`;
};

const renderFence = markdown.renderer.rules.fence;
markdown.renderer.rules.fence = (tokens, index, options, env, renderer) => {
  const token = tokens[index];
  if (token.info.trim().split(/\s+/, 1)[0] === 'mermaid' && env?.mermaid !== false) {
    const encoded = encodeURIComponent(token.content);
    return `<div class="mermaid-block" data-mermaid="${encoded}"><div class="mermaid-svg"></div></div>`;
  }
  return renderFence
    ? renderFence(tokens, index, options, env, renderer)
    : renderer.renderToken(tokens, index, options);
};

/** Image hosts used by LowNotes' own upload providers. They give uploaders no
 * access logs, so loading from them reveals nothing to the note's author. */
const TRUSTED_IMAGE_HOSTS = new Set(['files.catbox.moe', 'i.imgur.com']);

const remoteImagePolicy = { load: false, label: 'Remote image blocked. Click to load.' };

/** Set once from settings: whether remote images load without a click, and
 * the localized label of the placeholder shown instead. */
export function configureRemoteImages(options: { load?: boolean; label?: string }) {
  if (options.load !== undefined) remoteImagePolicy.load = options.load;
  if (options.label) remoteImagePolicy.label = options.label;
}

export function isRemoteImage(source: string): boolean {
  return /^https?:\/\//i.test(source);
}

function remoteImageAllowed(source: string, env: Record<string, unknown> | undefined): boolean {
  if (remoteImagePolicy.load || env?.remoteImages === true) return true;
  try {
    const url = new URL(source);
    if (url.protocol === 'https:' && TRUSTED_IMAGE_HOSTS.has(url.hostname)) return true;
  } catch { return false; }
  const allow = env?.allowRemoteImage;
  return typeof allow === 'function' && allow(source) === true;
}

const renderImage = markdown.renderer.rules.image;
markdown.renderer.rules.image = (tokens, index, options, env, renderer) => {
  const source = String(tokens[index].attrGet('src') ?? '');
  const resolve = env?.resolveImage;
  if (/^lownotes-image:[0-9a-f]{64}\.(png|jpg|gif|webp|bmp)$/.test(source) && typeof resolve === 'function') {
    const resolved = resolve(source);
    if (typeof resolved === 'string') tokens[index].attrSet('src', resolved);
  } else if (isRemoteImage(source) && !remoteImageAllowed(source, env)) {
    // Nothing is requested until the reader asks for this image.
    const escape = markdown.utils.escapeHtml;
    const alt = renderer.renderInlineAsText(tokens[index].children ?? [], options, env);
    const text = typeof env?.remoteImageLabel === 'string' && env.remoteImageLabel ? env.remoteImageLabel : remoteImagePolicy.label;
    const label = escape(alt ? `${text} (${alt})` : text);
    return `<button type="button" class="remote-image-blocked" data-remote-src="${escape(source)}" title="${escape(source)}">${label}</button>`;
  }
  return renderImage ? renderImage(tokens, index, options, env, renderer) : renderer.renderToken(tokens, index, options);
};

markdown.use(previewSourceMap);

export type RemoteImageOptions = { remoteImages?: boolean; allowRemoteImage?: (src: string) => boolean; remoteImageLabel?: string };

export function renderMarkdown(source: string, resolveImage?: (src: string) => string, options: TaskPreviewOptions & RemoteImageOptions & { sourceMap?: boolean | 'editor' } = {}): string {
  return markdown.render(source, { resolveImage, ...options, taskSource: source });
}

export function renderChatMarkdown(source: string): string {
  return markdown.render(source, { mermaid: false });
}
