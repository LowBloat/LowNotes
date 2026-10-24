import { expect, test } from 'bun:test';
import base from '../src-tauri/tauri.conf.json';
import mac from '../src-tauri/tauri.macos.conf.json';

// tauri.macos.conf.json replaces the whole windows array (JSON merge patch),
// so it must repeat every other window setting.
test('the macOS window differs only in its title bar', () => {
  const { decorations, titleBarStyle, hiddenTitle, ...macWindow } = mac.app.windows[0];
  const { decorations: baseDecorations, ...baseWindow } = base.app.windows[0];
  expect(macWindow).toEqual(baseWindow);
  expect(baseDecorations).toBe(false);
  expect({ decorations, titleBarStyle, hiddenTitle }).toEqual({ decorations: true, titleBarStyle: 'Overlay', hiddenTitle: true });
});

test('the main window starts hidden until the page is ready', () => {
  expect(base.app.windows[0].visible).toBe(false);
});
