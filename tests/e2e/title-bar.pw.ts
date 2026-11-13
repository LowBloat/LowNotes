import { test, expect } from '@playwright/test';
import { openVault } from './native-fixture';

test('the window buttons drive the native window', async ({ page }) => {
  const fixture = await openVault(page);
  try {
    const bar = page.locator('header.titlebar');
    await expect(bar).toContainText('Lista');
    await expect(bar).toContainText('Test vault');
    await bar.getByRole('button', { name: 'Minimize' }).click();
    await bar.getByRole('button', { name: 'Maximize' }).click();
    await bar.getByRole('button', { name: 'Close' }).click();
    await expect.poll(() => fixture.calls.map(call => call.command).filter(command => /minimize|toggle_maximize|\|close/.test(command)))
      .toEqual(['plugin:window|minimize', 'plugin:window|toggle_maximize', 'plugin:window|close']);
    expect(fixture.errors).toEqual([]);
  } finally { fixture.destroy(); }
});

test('the menu bar opens with the pointer and moves with the arrows', async ({ page }) => {
  const fixture = await openVault(page);
  try {
    const menubar = page.getByRole('menubar', { name: 'Application menu' });
    await menubar.getByRole('menuitem', { name: 'View' }).click();
    const menu = page.getByRole('menu', { name: 'View' });
    await expect(menu.getByRole('menuitemcheckbox', { name: 'Split' })).toHaveAttribute('aria-checked', 'true');
    await menu.getByRole('menuitemcheckbox', { name: 'Preview' }).click();
    await expect.poll(() => fixture.calls.find(call => call.command === 'save_view_mode')?.args).toEqual({ viewMode: 'preview' });

    await menubar.getByRole('menuitem', { name: 'File' }).click();
    await page.keyboard.press('ArrowDown');
    await expect(page.getByRole('menu', { name: 'File' }).getByRole('menuitem', { name: 'New Note' })).toBeFocused();
    await page.keyboard.press('ArrowRight');
    await expect(page.getByRole('menu', { name: 'Edit' }).getByRole('menuitem', { name: 'Undo' })).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(page.getByRole('menu')).toBeHidden();
    expect(fixture.errors).toEqual([]);
  } finally { fixture.destroy(); }
});

test('menu commands reach the editor and the sidebar', async ({ page }) => {
  const fixture = await openVault(page);
  try {
    const menubar = page.getByRole('menubar', { name: 'Application menu' });
    await menubar.getByRole('menuitem', { name: 'Edit' }).click();
    await page.getByRole('menu', { name: 'Edit' }).getByRole('menuitem', { name: 'Find in note' }).click();
    await expect(page.getByRole('search').or(page.getByPlaceholder(/Find/))).toBeVisible();

    await page.keyboard.press('Escape');
    await page.locator('.cm-content').click();
    await page.keyboard.press('Control+n');
    await expect(page.getByPlaceholder('Note name (e.g. Ideas)')).toBeFocused();

    await menubar.getByRole('menuitem', { name: 'Help' }).click();
    await page.getByRole('menu', { name: 'Help' }).getByRole('menuitem', { name: 'Report a problem' }).click();
    await expect.poll(() => fixture.openedUrls).toEqual(['https://github.com/LowBloat/LowNotes/issues/new/choose']);
    expect(fixture.errors).toEqual([]);
  } finally { fixture.destroy(); }
});

test('a new version stays on offer in the title bar after its dialog closes', async ({ page }) => {
  const fixture = await openVault(page, { update: { version: '9.9.9', body: 'Notes' } });
  try {
    const dialog = page.getByRole('dialog', { name: 'Update available' });
    await expect(dialog.getByRole('button', { name: 'Restart now' })).toBeVisible();
    await dialog.getByRole('button', { name: 'Later' }).click();
    await expect(dialog).toBeHidden();
    await page.locator('header.titlebar').getByRole('button', { name: 'Restart to update' }).click();
    await expect(dialog.getByRole('button', { name: 'Restart now' })).toBeVisible();
    expect(fixture.calls.filter(call => call.command === 'plugin:updater|check')[0]?.args).toMatchObject({ timeout: 30000 });
    expect(fixture.errors).toEqual([]);
  } finally { fixture.destroy(); }
});
