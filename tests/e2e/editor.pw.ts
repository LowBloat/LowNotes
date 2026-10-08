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
