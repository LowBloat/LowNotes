import { test, expect } from '@playwright/test';
import { openVault } from './native-fixture';

const plan = { path: 'Ideas/plan.md', name: 'plan.md', title: 'plan', is_dir: false, size: 1, modified_ms: 1 };
const ideas = { path: 'Ideas', name: 'Ideas', title: 'Ideas', is_dir: true, size: 0, modified_ms: 1 };

test('a note in the tree can be duplicated from its context menu', async ({ page }) => {
  const fixture = await openVault(page);
  try {
    await page.getByRole('button', { name: 'Lista (Lista.md)' }).click({ button: 'right' });
    const menu = page.getByRole('menu');
    await expect(menu.getByRole('menuitem', { name: 'Open' })).toBeFocused();
    await menu.getByRole('menuitem', { name: 'Duplicate' }).click();
    await expect(menu).toBeHidden();
    const dialog = page.getByRole('dialog');
    await expect(dialog.getByRole('textbox')).toHaveValue('Lista copy.md');
    await dialog.getByRole('button', { name: 'OK' }).click();
    await expect.poll(() => fixture.calls.find(call => call.command === 'duplicate_note')?.args).toEqual({ path: 'Lista.md', newPath: 'Lista copy.md' });
    await expect.poll(() => fixture.reads.at(-1)).toBe('Lista copy.md');
    expect(fixture.errors).toEqual([]);
  } finally { fixture.destroy(); }
});

test('the context menu works from the keyboard and returns the focus', async ({ page }) => {
  const fixture = await openVault(page, { otherNotes: [ideas, plan] });
  try {
    const folder = page.getByRole('button', { name: 'Ideas (Ideas)' });
    await folder.focus();
    await page.keyboard.press('Shift+F10');
    const menu = page.getByRole('menu');
    await expect(menu.getByRole('menuitem', { name: 'New note here' })).toBeFocused();
    await page.keyboard.press('ArrowUp');
    await expect(menu.getByRole('menuitem', { name: 'Delete' })).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(menu).toBeHidden();
    await expect(folder).toBeFocused();

    await folder.click({ button: 'right' });
    await menu.getByRole('menuitem', { name: 'Show in folder' }).click();
    await expect.poll(() => fixture.calls.find(call => call.command === 'show_in_folder')?.args).toEqual({ vaultId: 'fixture', path: 'Ideas' });

    await folder.click({ button: 'right' });
    await menu.getByRole('menuitem', { name: 'New note here' }).click();
    await expect(page.getByPlaceholder('Note name (e.g. Ideas)')).toBeFocused();
    await expect(page.getByPlaceholder('Note name (e.g. Ideas)')).toHaveValue('Ideas/');
    expect(fixture.errors).toEqual([]);
  } finally { fixture.destroy(); }
});

test('the editor menu formats the word under the pointer', async ({ page }) => {
  const fixture = await openVault(page, { source: 'plain words' });
  try {
    const content = page.locator('.cm-content');
    await content.dblclick({ position: { x: 4, y: 8 } });
    await content.click({ button: 'right', position: { x: 4, y: 8 } });
    const menu = page.getByRole('menu');
    await expect(menu.getByRole('menuitem', { name: 'Cut' })).toBeEnabled();
    await menu.getByRole('menuitem', { name: 'Bold' }).click();
    await expect.poll(() => fixture.content()).toBe('**plain** words');

    // The shortcuts the menu shows work too.
    await page.keyboard.press('End');
    await page.keyboard.press('Shift+Control+ArrowLeft');
    await page.keyboard.press('Control+i');
    await expect.poll(() => fixture.content()).toBe('**plain** *words*');
    expect(fixture.errors).toEqual([]);
  } finally { fixture.destroy(); }
});

test('preview links offer to open or copy their address', async ({ page }) => {
  const fixture = await openVault(page);
  try {
    await page.locator('.prose').getByRole('link', { name: 'Title' }).click({ button: 'right' });
    const menu = page.getByRole('menu');
    await expect(menu.getByRole('menuitem', { name: 'Copy link address' })).toBeVisible();
    await menu.getByRole('menuitem', { name: 'Open link' }).click();
    await expect.poll(() => fixture.openedUrls).toEqual(['https://visible.example']);
    expect(fixture.errors).toEqual([]);
  } finally { fixture.destroy(); }
});

test('areas without a menu of their own do not show the WebView menu', async ({ page }) => {
  const fixture = await openVault(page);
  try {
    const prevented = await page.locator('footer').first().evaluate((footer) => {
      const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
      footer.dispatchEvent(event);
      return event.defaultPrevented;
    });
    expect(prevented).toBe(true);
    const search = page.getByPlaceholder('Search notes...');
    expect(await search.evaluate((input) => {
      const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
      input.dispatchEvent(event);
      return event.defaultPrevented;
    })).toBe(false);
    expect(fixture.errors).toEqual([]);
  } finally { fixture.destroy(); }
});
