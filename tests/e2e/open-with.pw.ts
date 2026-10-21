import { test, expect } from '@playwright/test';
import { openVault } from './native-fixture';

const plan = { path: 'Ideas/plan.md', name: 'plan.md', title: 'plan', is_dir: false, size: 1, modified_ms: 1 };

test('a note from "Open with LowNotes" opens in the running window', async ({ page }) => {
  const fixture = await openVault(page, { otherNotes: [plan] });
  try {
    await fixture.requestOpen({ vault_path: 'fixture', note_path: 'Ideas/plan.md', known_vault: true });
    await expect.poll(() => fixture.reads.at(-1)).toBe('Ideas/plan.md');
    expect(fixture.calls.some(call => call.command === 'select_vault')).toBe(false);
    expect(fixture.errors).toEqual([]);
  } finally { fixture.destroy(); }
});

test('a folder outside the vault list opens only after confirmation', async ({ page }) => {
  const fixture = await openVault(page, { otherNotes: [plan] });
  try {
    const request = { vault_path: '/home/me/Loose', note_path: 'Ideas/plan.md', known_vault: false };
    await fixture.requestOpen(request);
    const dialog = page.getByRole('dialog');
    await expect(dialog).toContainText('“/home/me/Loose” is not a vault yet.');
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    expect(fixture.calls.some(call => call.command === 'select_vault')).toBe(false);

    await fixture.requestOpen(request);
    await page.getByRole('dialog').getByRole('button', { name: 'Open as vault' }).click();
    await expect.poll(() => fixture.calls.find(call => call.command === 'select_vault')?.args).toEqual({ pathStr: '/home/me/Loose' });
    await expect.poll(() => fixture.reads.at(-1)).toBe('Ideas/plan.md');
    expect(fixture.errors).toEqual([]);
  } finally { fixture.destroy(); }
});

test('the context menu preference is off by default and saves with the interface language', async ({ page }) => {
  const fixture = await openVault(page, { contextMenu: true });
  try {
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    const toggle = page.getByRole('checkbox', { name: /Show “Open with LowNotes”/ });
    await expect(toggle).not.toBeChecked();
    await toggle.check();
    await expect.poll(() => fixture.calls.find(call => call.command === 'save_context_menu')?.args).toEqual({ enabled: true, language: 'en-US' });
    await expect(toggle).toBeChecked();
    expect(fixture.errors).toEqual([]);
  } finally { fixture.destroy(); }
});

test('platforms without the context menu do not show the preference', async ({ page }) => {
  const fixture = await openVault(page);
  try {
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await expect(page.getByText('Keep running in the system tray')).toBeVisible();
    await expect(page.getByRole('checkbox', { name: /Open with LowNotes/ })).toHaveCount(0);
  } finally { fixture.destroy(); }
});
