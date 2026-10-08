import { test, expect } from '@playwright/test';
import { openVault } from './native-fixture';

test('versions compare and restore through a new native edit and update the open editor', async ({ page }) => {
  const fixture = await openVault(page, { source: '# Current\n\nCurrent paragraph.', history: { versionContent: '# Original\n\nOriginal paragraph.' } });
  await page.getByRole('button', { name: 'History and trash', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'History and trash' });
  await dialog.locator('.entry').first().click();
  await expect(dialog.getByRole('textbox', { name: 'Current Markdown', exact: true })).toHaveValue('# Current\n\nCurrent paragraph.');
  await expect(dialog.getByRole('textbox', { name: 'Previous or comparison Markdown' })).toHaveValue('# Original\n\nOriginal paragraph.');
  await dialog.getByRole('button', { name: 'Restore this version' }).click();
  await expect(dialog.getByRole('status')).toContainText('saved');
  expect(fixture.historyActions.find(action => action.command === 'history_apply')?.args).toMatchObject({ vaultId: 'fixture', noteId: 'fixture-note-id', versionId: '1'.repeat(64), content: null });
  await dialog.getByRole('button', { name: 'Close history' }).click();
  await expect(page.locator('.cm-content')).toContainText('Original paragraph.');
  expect(fixture.content()).toBe('# Original\n\nOriginal paragraph.');
  expect(fixture.errors).toEqual([]);
});

test('conflict comparison can combine texts and rejects stale application without discarding the result', async ({ page }) => {
  const fixture = await openVault(page, { source: 'Current content', history: { comparison: { path: 'conflict.md', content: 'Offline content' } } });
  await page.getByRole('button', { name: 'History and trash', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'History and trash' });
  await dialog.getByLabel('Compare with another note').selectOption('conflict.md');
  await expect(dialog.getByRole('textbox', { name: 'Previous or comparison Markdown' })).toHaveValue('Offline content');
  await dialog.getByRole('button', { name: 'Combine both' }).click();
  await expect(dialog.getByRole('textbox', { name: 'Result to apply' })).toHaveValue('Current content\n\nOffline content');
  await fixture.remoteAppend('\nRemote edit');
  await dialog.getByRole('button', { name: 'Apply result' }).click();
  await expect(dialog.getByRole('alert')).toContainText('changed during comparison');
  expect(fixture.content()).toBe('Current content\nRemote edit');
  await dialog.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(dialog.getByRole('textbox', { name: 'Current Markdown', exact: true })).toHaveValue('Current content\nRemote edit');
  await expect(dialog.getByRole('textbox', { name: 'Result to apply' })).toHaveValue('Current content\n\nOffline content');
  await dialog.getByRole('textbox', { name: 'Result to apply' }).fill('Current content\nRemote edit\nOffline content');
  await dialog.getByRole('button', { name: 'Apply result' }).click();
  await expect(dialog.getByRole('status')).toContainText('saved');
  expect(fixture.content()).toContain('Remote edit\nOffline content');
  expect(fixture.errors).toEqual([]);
});

test('trash restores a selected item to the backend-chosen path and refreshes the list', async ({ page }) => {
  const fixture = await openVault(page, { source: 'Keep current note', history: { trashContent: 'Deleted content' } });
  await page.getByRole('button', { name: 'History and trash', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'History and trash' });
  await dialog.getByRole('button', { name: 'Trash', exact: true }).click();
  await dialog.locator('.entry').first().click();
  await expect(dialog.getByRole('textbox', { name: 'Deleted Markdown' })).toHaveValue('Deleted content');
  await dialog.getByRole('button', { name: 'Restore item' }).click();
  await expect(dialog.getByRole('status')).toContainText('removed (restored).md');
  await expect(dialog).toContainText('no restorable items');
  expect(fixture.historyActions.find(action => action.command === 'history_trash_restore')?.args).toMatchObject({ recordId: '2'.repeat(64), noteId: '3'.repeat(64) });
  await dialog.getByRole('button', { name: 'Close history' }).click();
  await expect(page.locator('.cm-content')).toContainText('Deleted content');
  expect(fixture.content()).toBe('Keep current note');
  expect(fixture.errors).toEqual([]);
});

test('retention keeps forever by default and previews protected data before explicit cleanup', async ({ page }) => {
  const fixture = await openVault(page);
  await page.getByRole('button', { name: 'History and trash', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'History and trash' });
  await dialog.getByRole('button', { name: 'Retention', exact: true }).click();
  await expect(dialog.getByLabel('Keep versions for (days)')).toHaveValue('');
  await expect(dialog.getByLabel('Keep trash for (days)')).toHaveValue('');
  await dialog.getByRole('button', { name: 'Preview cleanup' }).click();
  await expect(dialog.getByRole('button', { name: 'Permanently remove expired data' })).toBeDisabled();
  await dialog.getByLabel('Keep versions for (days)').fill('30');
  await dialog.getByLabel('Keep trash for (days)').fill('90');
  await expect(dialog.getByRole('button', { name: 'Preview cleanup' })).toBeDisabled();
  await dialog.getByRole('button', { name: 'Save retention' }).click();
  await expect(dialog.getByRole('status')).toContainText('Retention saved');
  await dialog.getByRole('button', { name: 'Preview cleanup' }).click();
  await expect(dialog.getByRole('status')).toContainText('3 archives or paths protected');
  await dialog.getByRole('button', { name: 'Permanently remove expired data' }).click();
  await expect(dialog.getByRole('status').first()).toContainText('Removed 2');
  expect(fixture.historyActions.find(action => action.command === 'history_retention_save')?.args.policy).toEqual({ versions_days: 30, trash_days: 90 });
  expect(fixture.historyActions.filter(action => action.command === 'history_cleanup').map(action => action.args.apply)).toEqual([false, false, true]);
  expect(fixture.errors).toEqual([]);
});

for (const theme of ['light', 'dark'] as const) {
  test(`history comparison remains usable in ${theme} theme`, async ({ page }, info) => {
    await openVault(page, { theme, source: '# Current version\n\n- [x] Completed task\n\nText from this computer.', history: {
      versionContent: '# Previous version\n\n- [ ] Completed task\n\nText preserved before editing.' } });
    await page.getByRole('button', { name: 'History and trash', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'History and trash' });
    await dialog.locator('.entry').first().click();
    await expect(dialog.getByRole('textbox', { name: 'Current Markdown', exact: true })).toBeVisible();
    await expect(dialog.getByRole('textbox', { name: 'Previous or comparison Markdown' })).toBeVisible();
    await page.screenshot({ path: info.outputPath(`history-${theme}.png`) });
  });
}
