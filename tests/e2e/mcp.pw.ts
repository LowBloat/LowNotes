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
      .toEqual({ enabled: true, vaultIds: ['fixture'] });
    await expect(toggle).toBeChecked();
    await expect(page.getByRole('checkbox', { name: /Test vault/ })).toBeChecked();
    await expect(page.getByText('Running', { exact: true })).toBeVisible();
    await expect(page.getByText('2 requests since LowNotes started')).toBeVisible();

    const bridge = String.raw`C:\Program Files\LowNotes\lownotes-mcp.exe`;
    await expect(page.getByText(`claude mcp add --scope user lownotes -- "${bridge}"`)).toBeVisible();
    await expect(page.locator('.mcp-snippet pre').first()).toContainText(JSON.stringify(bridge));

    await page.getByRole('checkbox', { name: /Test vault/ }).uncheck();
    await expect.poll(() => fixture.calls.filter(call => call.command === 'save_mcp_settings').at(-1)?.args)
      .toEqual({ enabled: true, vaultIds: [] });

    await page.getByRole('button', { name: 'Regenerate token' }).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Regenerate token' }).click();
    await expect.poll(() => fixture.calls.some(call => call.command === 'mcp_regenerate_token')).toBe(true);
    expect(fixture.errors).toEqual([]);
  } finally { fixture.destroy(); }
});
