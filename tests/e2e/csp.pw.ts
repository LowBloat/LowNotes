import { test, expect } from '@playwright/test';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { openVault } from './native-fixture';

/** Serve the built app with the CSP from tauri.conf.json, the way Tauri does:
 * inline scripts in index.html get their sha256 hash added to script-src
 * (style-src is excluded from that modification in the config). */
const config = JSON.parse(readFileSync(new URL('../../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
const csp: Record<string, string> = config.app.security.csp;

function policyFor(html: string): string {
  const hashes = [...html.matchAll(/<script(?![^>]*\bsrc=)[^>]*>([\s\S]*?)<\/script>/g)]
    .map(([, body]) => `'sha256-${createHash('sha256').update(body).digest('base64')}'`);
  return Object.entries(csp)
    .map(([directive, value]) => directive === 'script-src' ? `${directive} ${value} ${hashes.join(' ')}` : `${directive} ${value}`)
    .join('; ');
}

test('the app runs under the production CSP without violations', async ({ page }) => {
  await page.route('**/*', async (route) => {
    const request = route.request();
    if (request.resourceType() !== 'document') return route.continue();
    const response = await route.fetch();
    const html = await response.text();
    await route.fulfill({ response, body: html, headers: { ...response.headers(), 'content-security-policy': policyFor(html) } });
  });
  await page.addInitScript(() => {
    (window as any).__cspViolations = [];
    document.addEventListener('securitypolicyviolation', (event) => {
      (window as any).__cspViolations.push(`${event.violatedDirective} ${event.blockedURI}`);
    });
  });
  const vault = await openVault(page, {
    source: '# CSP\n\n```mermaid\ngraph TD\n  A[Start] --> B{Choice}\n  B --> C[Done]\n```\n\n'
      + '```js\nconst answer = 42;\n```\n\n![remote](https://tracker.example/pixel.png)\n\n- [ ] task\n\n'
      + '```excalidraw\n{"elements": [{"id": "t", "type": "text", "x": 0, "y": 0, "text": "Hand-drawn", "originalText": "Hand-drawn", "fontSize": 20, "fontFamily": 5, "width": 110, "height": 25, "lineHeight": 1.25}]}\n```\n',
  });
  try {
    await expect(page.locator('.mermaid-svg svg')).toBeVisible();
    await expect(page.locator('.drawing-image img')).toBeVisible();
    await page.locator('article .drawing-block').click();
    await expect(page.locator('.excalidraw canvas.interactive')).toBeVisible();
    await page.getByRole('button', { name: 'Done' }).click();
    await expect(page.locator('article .remote-image-blocked')).toBeVisible();
    await page.locator('.cm-content').focus();
    await page.keyboard.type(' more text');
    await expect(page.locator('article')).toContainText('more text');
    const violations = await page.evaluate(() => (window as any).__cspViolations as string[]);
    expect(violations).toEqual([]);
    expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});

test('blocked remote images load only after a click', async ({ page }) => {
  const requested: string[] = [];
  await page.route('https://tracker.example/**', (route) => { requested.push(route.request().url()); return route.fulfill({ status: 204 }); });
  const vault = await openVault(page, { source: '# Images\n\n![pixel](https://tracker.example/pixel.png)\n' });
  try {
    const placeholder = page.locator('article .remote-image-blocked');
    await expect(placeholder).toContainText('pixel');
    await page.waitForTimeout(300);
    expect(requested).toEqual([]);
    await placeholder.click();
    await expect(page.locator('article img[src="https://tracker.example/pixel.png"]')).toHaveCount(1);
    await expect.poll(() => requested).toEqual(['https://tracker.example/pixel.png']);
    expect(vault.content()).toBe(vault.source);
    expect(vault.errors).toEqual([]);
  } finally { vault.destroy(); }
});
