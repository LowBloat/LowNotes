import type { MarkdownIt } from 'markdown-it';

/** Source locations are opt-in for the note preview; chat and exports retain their original markup. */
export function previewSourceMap(md: MarkdownIt) {
  md.core.ruler.after('lownotes-task-positions', 'lownotes-preview-source-map', state => {
    if (!state.env?.sourceMap) return;
    const original = typeof state.env.taskSource === 'string' ? state.env.taskSource : state.src;
    // CodeMirror stores line breaks as LF even when the original file uses CRLF.
    const source = state.env.sourceMap === 'editor' ? original.replace(/\r\n|\r/g, '\n') : original;
    const starts = [0];
    for (const match of source.matchAll(/\r\n|\n|\r/g)) starts.push(match.index! + match[0].length);
    state.env.sourceLineStarts = starts;
    state.env.sourceMapLength = source.length;
    const parents: ([number, number] | null)[] = [];
    for (const token of state.tokens) {
      if (token.nesting === -1) parents.pop();
      if (token.nesting === 1) parents.push(token.map);
      const map = token.map ?? [...parents].reverse().find(parent => parent !== null);
      if (token.type !== 'inline' || !map || !token.children) continue;
      const open = new state.Token('html_inline', '', 0);
      open.content = `<span data-search-from="${starts[map[0]] ?? 0}" data-search-to="${starts[map[1]] ?? source.length}">`;
      const close = new state.Token('html_inline', '', 0);
      close.content = '</span>';
      token.children.unshift(open);
      token.children.push(close);
    }
  });
  for (const name of ['fence', 'code_block']) {
    const render = md.renderer.rules[name];
    md.renderer.rules[name] = (tokens, index, options, env, renderer) => {
      const html = render ? render(tokens, index, options, env, renderer) : renderer.renderToken(tokens, index, options);
      const map = tokens[index].map;
      if (!env?.sourceMap || !map) return html;
      const starts = env.sourceLineStarts as number[];
      const length = env.sourceMapLength as number;
      return `<div data-search-from="${starts[map[0]] ?? 0}" data-search-to="${starts[map[1]] ?? length}">${html}</div>`;
    };
  }
}
