import { test, expect } from '@playwright/test';
import type { VaultItem } from '../../src/lib/types';
import { openVault } from './native-fixture';

const entry = (path: string, is_dir: boolean): VaultItem => {
  const name = path.split('/').at(-1)!;
  return { path, name, title: name.replace(/\.md$/, ''), is_dir, size: 1, modified_ms: 1 };
};

test('folders open the way they were left and each toggle is remembered', async ({ page }) => {
  const fixture = await openVault(page, {
    collapsedFolders: ['Archive'],
    otherNotes: [entry('Archive', true), entry('Archive/Old.md', false), entry('Projects', true), entry('Projects/Plan.md', false)],
  });
  const saved = () => fixture.calls.filter(call => call.command === 'save_collapsed_folders').at(-1)?.args;
  try {
    const sidebar = page.locator('aside');
    const archive = sidebar.getByRole('button', { name: 'Archive (Archive)' });
    await expect(archive).toHaveAttribute('aria-expanded', 'false');
    await expect(sidebar.getByRole('button', { name: 'Old (Archive/Old.md)' })).toHaveCount(0);
    await expect(sidebar.getByRole('button', { name: 'Plan (Projects/Plan.md)' })).toBeVisible();
    expect(saved()).toBeUndefined();

    await sidebar.getByRole('button', { name: 'Projects (Projects)' }).click();
    await expect(sidebar.getByRole('button', { name: 'Plan (Projects/Plan.md)' })).toHaveCount(0);
    await expect.poll(saved).toEqual({ vaultId: 'fixture', folders: ['Archive', 'Projects'] });

    await archive.click();
    await expect(sidebar.getByRole('button', { name: 'Old (Archive/Old.md)' })).toBeVisible();
    await expect.poll(saved).toEqual({ vaultId: 'fixture', folders: ['Projects'] });
    expect(fixture.errors).toEqual([]);
  } finally { fixture.destroy(); }
});
