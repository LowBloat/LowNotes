import { test, expect, type Page } from '@playwright/test';
import { openVault } from './native-fixture';

const rectangle = (id: string, x: number, index: string) => JSON.stringify({ id, type: 'rectangle', x, y: 0, width: 120, height: 70, angle: 0,
  strokeColor: '#1e1e1e', backgroundColor: 'transparent', fillStyle: 'solid', strokeWidth: 2, strokeStyle: 'solid', roughness: 1, opacity: 100,
  groupIds: [], frameId: null, roundness: null, seed: 1, version: 1, versionNonce: 1, isDeleted: false, boundElements: null, updated: 1, link: null, locked: false, index });
const drawing = (...elements: string[]) => `{\n"type": "excalidraw",\n"version": 2,\n"source": "lownotes",\n"elements": [\n${elements.join(',\n')}\n],\n"appState": {"viewBackgroundColor":"#ffffff"},\n"files": {\n}\n}\n`;
const ids = (content: string): string[] => {
  const block = /```excalidraw\n([\s\S]*?)```/.exec(content)?.[1];
  return block ? JSON.parse(block).elements.filter((element: { isDeleted: boolean }) => !element.isDeleted).map((element: { id: string }) => element.id) : [];
};

async function drawRectangle(page: Page, from: [number, number], to: [number, number]) {
  const canvas = page.locator('.excalidraw canvas.interactive');
  const box = (await canvas.boundingBox())!;
  await page.keyboard.press('r');
  await page.mouse.move(box.x + from[0], box.y + from[1]);
  await page.mouse.down();
  await page.mouse.move(box.x + to[0], box.y + to[1], { steps: 5 });
  await page.mouse.up();
}

test('a drawing in a note renders in the preview and saves strokes back into the note', async ({ page }) => {
  const vault = await openVault(page, { source: `# Plan\n\n\`\`\`excalidraw\n${drawing(rectangle('first', 0, 'a0'))}\`\`\`\n\nAfter\n` });
  try {
    await expect(page.locator('article .drawing-image img')).toBeVisible();
    // The JSON is folded in the editor pane.
    await expect(page.locator('.cm-foldPlaceholder')).toHaveCount(1);
    await expect(page.locator('.cm-content')).not.toContainText('"rectangle"');
    await page.locator('article .drawing-block').click();
    const editor = page.getByRole('dialog', { name: 'Drawing' });
    await expect(editor.locator('.excalidraw canvas.interactive')).toBeVisible();
    await drawRectangle(page, [500, 300], [650, 400]);
    await expect.poll(() => ids(vault.content()).length).toBe(2);
    expect(ids(vault.content())[0]).toBe('first');
    expect(vault.content()).toMatch(/^# Plan\n\n```excalidraw\n[\s\S]*\n```\n\nAfter\n$/);
    await editor.getByRole('button', { name: 'Done' }).click();
    await expect(editor).toHaveCount(0);
    await expect(page.locator('article .drawing-image img')).toBeVisible();
    expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});

test('the toolbar adds a drawing at the cursor, and one closed without strokes leaves nothing behind', async ({ page }) => {
  const vault = await openVault(page, { source: '# Sketch\n\nText\n' });
  try {
    await page.locator('.cm-content').focus();
    await page.keyboard.press('Control+End');
    await page.getByRole('button', { name: 'Drawing' }).click();
    const editor = page.getByRole('dialog', { name: 'Drawing' });
    await expect(editor.locator('.excalidraw canvas.interactive')).toBeVisible();
    await expect.poll(vault.content).toContain('```excalidraw');
    await editor.getByRole('button', { name: 'Done' }).click();
    await expect.poll(vault.content).toBe('# Sketch\n\nText\n');

    await page.getByRole('button', { name: 'Drawing' }).click();
    await drawRectangle(page, [400, 300], [520, 380]);
    await expect.poll(() => ids(vault.content()).length).toBe(1);
    await editor.getByRole('button', { name: 'Done' }).click();
    await expect(page.locator('article .drawing-image img')).toBeVisible();
    expect(vault.content()).toMatch(/^# Sketch\n\nText\n\n```excalidraw\n[\s\S]*\n```\n$/);
    expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});

test('strokes from another device merge into the open drawing instead of being overwritten', async ({ page }) => {
  const vault = await openVault(page, { source: `\`\`\`excalidraw\n${drawing(rectangle('first', 0, 'a0'))}\`\`\`\n` });
  try {
    await page.locator('article .drawing-block').click();
    const editor = page.getByRole('dialog', { name: 'Drawing' });
    await expect(editor.locator('.excalidraw canvas.interactive')).toBeVisible();
    await vault.remoteEdit((text) => {
      const anchor = text.toString().indexOf('\n],');
      text.insert(anchor, `,\n${rectangle('remote', 300, 'a1')}`);
    });
    await drawRectangle(page, [500, 400], [600, 480]);
    await expect.poll(() => ids(vault.content()).sort()).toHaveLength(3);
    expect(ids(vault.content())).toEqual(expect.arrayContaining(['first', 'remote']));
    expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});
