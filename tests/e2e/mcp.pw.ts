import { test, expect } from '@playwright/test';
import { openVault } from './native-fixture';

test('turning on the MCP server shares the open vault and shows how to connect clients', async ({ page }) => {
  const fixture = await openVault(page);
  try {
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('button', { name: 'MCP server', exact: true }).click();
    const toggle = page.getByRole('checkbox', { name: /Turn on the MCP server/ });
    await expect(toggle).not.toBeChecked();
    await toggle.check();
    await expect.poll(() => fixture.calls.find(call => call.command === 'save_mcp_settings')?.args)
      .toEqual({ enabled: true, vaultIds: ['fixture'], allowChanges: false });
    await expect(toggle).toBeChecked();
    await expect(page.getByRole('checkbox', { name: /Test vault/ })).toBeChecked();
    await expect(page.getByText('Running', { exact: true })).toBeVisible();
    await expect(page.getByText('2 requests since LowNotes started')).toBeVisible();

    const bridge = String.raw`C:\Program Files\LowNotes\lownotes-mcp.exe`;
    await expect(page.getByText(`claude mcp add --scope user lownotes -- "${bridge}"`)).toBeVisible();
    await expect(page.locator('.mcp-snippet pre').first()).toContainText(JSON.stringify(bridge));

    await page.getByRole('checkbox', { name: /Test vault/ }).uncheck();
    await expect.poll(() => fixture.calls.filter(call => call.command === 'save_mcp_settings').at(-1)?.args)
      .toEqual({ enabled: true, vaultIds: [], allowChanges: false });

    await page.getByRole('button', { name: 'Regenerate token' }).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Regenerate token' }).click();
    await expect.poll(() => fixture.calls.some(call => call.command === 'mcp_regenerate_token')).toBe(true);
    expect(fixture.errors).toEqual([]);
  } finally { fixture.destroy(); }
});

test('AI apps may change notes only after the user confirms it', async ({ page }) => {
  const fixture = await openVault(page);
  const saved = () => fixture.calls.filter(call => call.command === 'save_mcp_settings').map(call => call.args);
  try {
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('button', { name: 'MCP server', exact: true }).click();
    const allow = page.getByRole('checkbox', { name: /Allow changes/ });
    await expect(allow).toBeDisabled();
    await page.getByRole('checkbox', { name: /Turn on the MCP server/ }).check();
    await expect(allow).toBeEnabled();

    await allow.click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toContainText('Notes synced from other devices');
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    await expect(allow).not.toBeChecked();
    expect(saved()).toHaveLength(1);

    await allow.click();
    await dialog.getByRole('button', { name: 'Allow changes' }).click();
    await expect.poll(() => saved().at(-1)).toEqual({ enabled: true, vaultIds: ['fixture'], allowChanges: true });
    await expect(allow).toBeChecked();

    // Sharing another vault keeps the permission; turning it off needs no confirmation.
    await page.getByRole('checkbox', { name: /Test vault/ }).uncheck();
    await expect.poll(() => saved().at(-1)).toEqual({ enabled: true, vaultIds: [], allowChanges: true });
    await allow.uncheck();
    await expect.poll(() => saved().at(-1)).toEqual({ enabled: true, vaultIds: [], allowChanges: false });
    await expect(dialog).toHaveCount(0);
    expect(fixture.errors).toEqual([]);
  } finally { fixture.destroy(); }
});

test('notes an AI app creates, moves or deletes show up in the open vault', async ({ page }) => {
  const fixture = await openVault(page);
  const listings = () => fixture.calls.filter(call => call.command === 'list_notes').length;
  try {
    const before = listings();
    await fixture.emit('vault:items-changed', { vault_id: 'another-vault' });
    await fixture.emit('vault:items-changed', { vault_id: 'fixture' });
    await expect.poll(listings).toBe(before + 1);
    // The event for another vault must not have reloaded the list as well.
    await page.waitForTimeout(300);
    expect(listings()).toBe(before + 1);
    expect(fixture.errors).toEqual([]);
  } finally { fixture.destroy(); }
});
