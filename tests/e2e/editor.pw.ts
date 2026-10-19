import { test, expect } from '@playwright/test';
import { openVault } from './native-fixture';
const MOD = process.platform === 'darwin' ? 'Meta' : 'Control';

test('a stale editor sends its full recovery snapshot with the original note and vault identities', async ({ page }) => {
  const vault = await openVault(page, { source: '# Original\n\ntext\n' });
  try {
    vault.deleteWhileEditing();
    await page.locator('.cm-content').focus(); await page.keyboard.press(`${MOD}+End`); await page.keyboard.type('late edit');
    await expect.poll(() => vault.recovered.at(-1)).toContain('late edit');
    expect(vault.content()).toBe(vault.source);
    expect(vault.saves.some(save => save.noteId === 'fixture-note-id' && save.vaultId === 'fixture' && save.recoveryUpdate === true)).toBe(true);
    expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});

test('task toggles persist, synchronize, and undo without undoing remote text', async ({ page }) => {
  const vault = await openVault(page, { source: '# Tasks\n\n- [ ] First task\n- [ ] Second task\n' });
  try {
    await page.locator('article input[type=checkbox]').first().check();
    await expect.poll(vault.content).toContain('- [x] First task');
    await vault.remoteAppend('Remote text 🙂');
    await expect(page.locator('article')).toContainText('Remote text 🙂');
    await page.locator('.cm-content').focus();
    await page.keyboard.press(`${MOD}+z`);
    await expect.poll(vault.content).toContain('- [ ] First task');
    expect(vault.content()).toContain('Remote text 🙂');
    await page.keyboard.press(`${MOD}+Shift+Z`);
    await expect.poll(vault.content).toContain('- [x] First task');
    await page.reload();
    await expect(page.locator('article input[type=checkbox]').first()).toBeChecked();
    await expect(page.locator('article')).toContainText('Remote text 🙂');
    expect(vault.saves.length).toBeGreaterThan(0);
    expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});

test('floating search follows each mode and navigates both panes without editing', async ({ page }) => {
  const vault = await openVault(page);
  try {
    await page.getByPlaceholder('Search notes...').focus();
    await page.keyboard.press(`${MOD}+f`);
    const panel = page.getByRole('region', { name: 'Find in note' });
    const input = panel.getByRole('textbox', { name: 'Find in note…' });
    const counter = panel.locator('.find-count');
    await expect(input).toBeFocused();
    const before = await panel.boundingBox();
    await input.fill('needle');
    await expect(counter).toHaveText('1 / 14');
    await expect(page.locator('article mark[data-note-search]')).toHaveCount(14);
    await input.press('F3'); await expect(counter).toHaveText('2 / 14');
    await input.press('Shift+F3'); await expect(counter).toHaveText('1 / 14');
    for (let i = 0; i < 8; i++) await input.press('Enter');
    await expect(counter).toHaveText('9 / 14');
    await expect.poll(() => page.locator('.cm-scroller').evaluate(element => element.scrollTop)).toBeGreaterThan(0);
    await expect.poll(() => page.locator('article').evaluate(element => element.parentElement!.scrollTop)).toBeGreaterThan(0);
    expect(await panel.boundingBox()).toEqual(before);
    await panel.getByRole('button', { name: 'Use regular expression', exact: true }).click();
    await input.fill('[');
    await expect(panel.getByRole('status')).toHaveText('Invalid regular expression. Check the pattern.');
    await panel.getByRole('button', { name: 'Use regular expression', exact: true }).click();
    await input.fill('visible.example');
    await expect(counter).toHaveText('1 / 1');
    await page.getByRole('button', { name: 'Preview', exact: true }).click();
    await expect(counter).toHaveText('0 / 0');
    await page.getByRole('button', { name: 'Editor', exact: true }).click();
    await expect(counter).toHaveText('1 / 1');
    await page.getByRole('button', { name: 'Split', exact: true }).click();
    await input.fill('hello world');
    await expect(counter).toHaveText('1 / 1');
    await expect(page.locator('article mark[data-note-search]')).toHaveCount(2);
    await input.press('Escape');
    await expect(panel).toHaveCount(0);
    expect(vault.content()).toBe(vault.source);
    expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});

test('Ctrl-click delegates links to the system browser and ignores code examples', async ({ page }) => {
  const vault = await openVault(page, { source: '# Links\n\n[Title](https://visible.example)\n\n```text\nhttps://blocked.example\n```\n' });
  try {
    const link = page.locator('.cm-line').filter({ hasText: '[Title]' });
    await link.click({ position: { x: 28, y: 8 } });
    expect(vault.openedUrls).toEqual([]);
    await link.click({ modifiers: ['Control'], position: { x: 28, y: 8 } });
    await expect.poll(() => vault.openedUrls).toEqual(['https://visible.example/']);
    await page.locator('.cm-line').filter({ hasText: 'https://blocked.example' }).click({ modifiers: ['Control'], position: { x: 28, y: 8 } });
    expect(vault.openedUrls).toEqual(['https://visible.example/']);
    expect(vault.content()).toBe(vault.source);
    expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});

test('preview resolves encoded relative Markdown paths and vault wikilinks while web Markdown URLs use the browser', async ({ page }) => {
  const note = (path: string) => ({ path, name: path, title: path, is_dir: false, size: 0, modified_ms: 1 });
  const vault = await openVault(page, { notePath: 'folder/source.md',
    source: '[relative](sub/a%C3%A7%C3%A3o.md#etapa)\n\n[[sub/ação|wiki]]\n\n[external](https://visible.example/file.md)\n',
    otherNotes: [note('folder/sub/ação.md'), note('sub/ação.md')],
  });
  try {
    await page.getByRole('link', { name: 'relative', exact: true }).click();
    await expect.poll(() => vault.reads.at(-1)).toBe('folder/sub/ação.md');
    await page.getByRole('link', { name: 'wiki', exact: true }).click();
    await expect.poll(() => vault.reads.at(-1)).toBe('sub/ação.md');
    const reads = vault.reads.length;
    await page.getByRole('link', { name: 'external', exact: true }).click();
    await expect.poll(() => vault.openedUrls).toEqual(['https://visible.example/file.md']);
    expect(vault.reads).toHaveLength(reads);
    expect(vault.content()).toBe(vault.source); expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});

test('preview keeps the app in place for mail, relative files and footnotes', async ({ page }) => {
  const vault = await openVault(page, {
    source: '# Links\n\n[mail](mailto:user@example.org) [file](./report.pdf) [ftp](ftp://example.org/a) text[^1]\n\n'
      + 'filler\n\n'.repeat(80) + '[^1]: The footnote.\n',
  });
  try {
    const url = page.url();
    await page.getByRole('link', { name: 'mail', exact: true }).click();
    await expect.poll(() => vault.openedUrls).toEqual(['mailto:user@example.org']);
    await page.getByRole('link', { name: 'file', exact: true }).click();
    await page.getByRole('link', { name: 'ftp', exact: true }).click();
    await page.locator('article sup a').first().click();
    await expect(page.locator('article .footnotes')).toBeInViewport();
    expect(page.url()).toBe(url);
    expect(vault.openedUrls).toEqual(['mailto:user@example.org']);
    await expect(page.locator('article h1')).toHaveText('Links');
    expect(vault.content()).toBe(vault.source); expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});

for (const palette of ['lowbloat', 'megumin', 'rimuru']) {
  for (const theme of ['light', 'dark'] as const) {
    test(`links and code language labels remain readable in ${palette}/${theme}`, async ({ page }) => {
      const vault = await openVault(page, { palette, theme, source: '# Theme\n\n[Link](https://visible.example)\n\n```rust\nfn main() {}\n```\n' });
      try {
        await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
        const contrast = await page.locator('article a').evaluate(element => {
          const rgb = (value: string) => value.match(/[\d.]+/g)!.slice(0, 3).map(Number);
          const luminance = (values: number[]) => values.map(value => {
            const c = value / 255; return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
          }).reduce((sum, value, index) => sum + value * [0.2126, 0.7152, 0.0722][index], 0);
          const foreground = luminance(rgb(getComputedStyle(element).color));
          const background = luminance(rgb(getComputedStyle(document.body).backgroundColor));
          return (Math.max(foreground, background) + 0.05) / (Math.min(foreground, background) + 0.05);
        });
        expect(contrast).toBeGreaterThanOrEqual(4.5);
        const label = page.locator('.cm-line').filter({ hasText: '```rust' });
        await expect(label).toContainText('rust');
        // The configured language label token must inherit the palette, never browser blue.
        const color = await label.evaluate(element => {
          const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
          for (let node = walker.nextNode(); node; node = walker.nextNode()) {
            if (node.textContent?.includes('rust')) return getComputedStyle(node.parentElement!).color;
          }
          return null;
        });
        const accent = await page.locator('body').evaluate(element => {
          const probe = document.createElement('span'); probe.style.color = 'var(--accent-light)'; element.append(probe);
          const color = getComputedStyle(probe).color; probe.remove(); return color;
        });
        expect(color).toBe(accent);
        expect(vault.errors).toEqual([]);
      } finally { vault.destroy(); }
    });
  }
}

test('startup recovery is visible and can be dismissed without changing the note', async ({ page }) => {
  const vault = await openVault(page, { notices: [{ path: 'fixture/settings.json', recovered: true }] });
  try {
    await expect(page.getByText('fixture/settings.json', { exact: true })).toBeVisible();
    await page.getByText('fixture/settings.json', { exact: true }).locator('..').getByRole('button').click();
    await expect(page.getByText('fixture/settings.json', { exact: true })).toHaveCount(0);
    expect(vault.content()).toBe(vault.source);
    expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});

test('an unavailable credential store explains the issue and can be retried while notes remain usable', async ({ page }) => {
  const vault = await openVault(page, { credentialError: 'credentials.unavailable' });
  try {
    await expect(page.getByRole('alert')).toContainText('system credential store is unavailable');
    await expect(page.locator('.cm-content')).toBeVisible();
    await page.getByRole('button', { name: 'Open settings', exact: true }).click();
    await page.getByRole('button', { name: 'Try credential store again', exact: true }).click();
    await expect(page.getByText('Provider keys and device identity are stored in the system credential store.', { exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Try credential store again', exact: true })).toHaveCount(0);
    expect(vault.content()).toBe(vault.source);
    expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});

test('typing sends merged updates in a few calls and the note is read once', async ({ page }) => {
  const vault = await openVault(page, { source: '# Batch\n\n' });
  try {
    expect(vault.reads.filter((path) => path === 'Lista.md')).toHaveLength(1);
    await page.locator('.cm-content').focus();
    await page.keyboard.press(`${MOD}+End`);
    const sentence = 'Typing a whole sentence should not cost one native call per key.';
    await page.keyboard.type(sentence);
    await expect.poll(vault.content).toContain(sentence);
    expect(vault.saves.length).toBeGreaterThan(0);
    expect(vault.saves.length).toBeLessThan(sentence.length / 3);
    await expect(page.locator('article')).toContainText(sentence);
    expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});

test('edits typed right before switching notes are still saved', async ({ page }) => {
  const other = { path: 'Other.md', name: 'Other.md', title: 'Other', is_dir: false, size: 0, modified_ms: 1 };
  const vault = await openVault(page, { source: '# Switch\n\n', otherNotes: [other] });
  try {
    await page.locator('.cm-content').focus();
    await page.keyboard.press(`${MOD}+End`);
    await page.keyboard.type('last words');
    await page.getByRole('button', { name: 'Other (Other.md)' }).click();
    await expect.poll(() => vault.reads.at(-1)).toBe('Other.md');
    await expect.poll(vault.content).toContain('last words');
    expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});

test('every conflict copy stays listed until it is opened or dismissed', async ({ page }) => {
  const vault = await openVault(page);
  try {
    await vault.emit('p2p:conflict', { type: 'Conflict', note_path: 'Lista.md', conflict_path: 'Lista (conflict 1).md' });
    await vault.emit('p2p:conflict', { type: 'Conflict', note_path: 'Other.md', conflict_path: 'Other (conflict 2).md' });
    const alerts = page.getByRole('alert').filter({ hasText: 'conflict' });
    await expect(alerts).toHaveCount(2);
    await alerts.filter({ hasText: 'Other (conflict 2).md' }).getByRole('button', { name: 'Dismiss' }).click();
    await expect(alerts).toHaveCount(1);
    await expect(alerts).toContainText('Lista (conflict 1).md');
    expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});

test('update notes render as Markdown and their links open in the system browser', async ({ page }) => {
  const body = '## ✨ Features\n\n- **Remove vaults** — see [the release](https://github.com/LowBloat/LowNotes/releases).\n- [Local](./notes.md)\n';
  const vault = await openVault(page, { update: { version: '9.9.9', body } });
  try {
    const notes = page.getByTestId('release-notes');
    await expect(notes.getByRole('heading', { name: '✨ Features' })).toBeVisible();
    await expect(notes.locator('strong')).toHaveText('Remove vaults');
    await expect(notes).not.toContainText('**');
    const url = page.url();
    await notes.getByRole('link', { name: 'the release' }).click();
    await expect.poll(() => vault.openedUrls).toEqual(['https://github.com/LowBloat/LowNotes/releases']);
    await notes.getByRole('link', { name: 'Local' }).click();
    expect(vault.openedUrls).toEqual(['https://github.com/LowBloat/LowNotes/releases']);
    expect(page.url()).toBe(url);
    expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});
