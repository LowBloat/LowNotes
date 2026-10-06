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

const renderImage = markdown.renderer.rules.image;
markdown.renderer.rules.image = (tokens, index, options, env, renderer) => {
  const source = String(tokens[index].attrGet('src') ?? '');
  const resolve = env?.resolveImage;
  if (/^lownotes-image:[0-9a-f]{64}\.(png|jpg|gif|webp|bmp)$/.test(source) && typeof resolve === 'function') {
    const resolved = resolve(source);
    if (typeof resolved === 'string') tokens[index].attrSet('src', resolved);
  }
  return renderImage ? renderImage(tokens, index, options, env, renderer) : renderer.renderToken(tokens, index, options);
};

export function renderMarkdown(source: string, resolveImage?: (src: string) => string, options: TaskPreviewOptions = {}): string {
  return markdown.render(source, { resolveImage, ...options, taskSource: source });
}

export function renderChatMarkdown(source: string): string {
  return markdown.render(source, { mermaid: false });
}
