import { test, expect } from '@playwright/test';
import { openVault } from './native-fixture';

test('modals take focus, keep Tab inside, have a name and return focus on Escape', async ({ page }) => {
  const vault = await openVault(page);
  try {
    const trigger = page.getByRole('button', { name: 'Manage Connections' });
    await trigger.focus();
    await trigger.press('Enter');
    const dialog = page.getByRole('dialog', { name: 'P2P Sync & Devices' });
    await expect(dialog).toBeVisible();
    await expect.poll(() => dialog.evaluate((node) => node.contains(document.activeElement))).toBe(true);
    for (let i = 0; i < 25; i++) {
      await page.keyboard.press(i % 3 ? 'Tab' : 'Shift+Tab');
      expect(await dialog.evaluate((node) => node.contains(document.activeElement))).toBe(true);
    }
    await page.keyboard.press('Escape');
    await expect(dialog).toBeHidden();
    await expect(trigger).toBeFocused();
    expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});

test('the vault menu closes on Escape and outside clicks, and note rows are plain buttons', async ({ page }) => {
  const vault = await openVault(page);
  try {
    const toggle = page.getByRole('button', { name: 'Test vault' });
    await toggle.click();
    await expect(page.getByText('Your Vaults')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.getByText('Your Vaults')).toBeHidden();
    await expect(toggle).toBeFocused();
    await toggle.click();
    await page.locator('.cm-content').click();
    await expect(page.getByText('Your Vaults')).toBeHidden();
    // No interactive control is nested inside another one.
    const nested = await page.locator('button button, [role="button"] button, a button').count();
    expect(nested).toBe(0);
    expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});
