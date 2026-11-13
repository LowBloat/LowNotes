import { expect, test } from 'bun:test';
import type { Update } from '@tauri-apps/plugin-updater';
import { CHECK_TIMEOUT_MS, canInstallUpdate, checkAvailableUpdate, discardUpdate, updateInstructionKey } from '../src/lib/updates';
import type { UpdateChannel, UpdatePolicy } from '../src/lib/types';

function policy(channel: UpdateChannel, canInstall = false): UpdatePolicy {
  return { channel, can_install: canInstall,
    updater_target: channel === 'appimage' && canInstall ? 'linux-x86_64-appimage' : null };
}

test('managed Linux installations never call the native updater', async () => {
  for (const channel of ['arch', 'deb', 'rpm', 'flatpak', 'snap', 'portable', 'linux_package', 'appimage'] as const) {
    let nativeCalls = 0;
    let externalCalls = 0;
    const notice = { version: '0.2.7', body: 'Notes', download_url: null };
    const update = await checkAvailableUpdate(policy(channel),
      async () => { nativeCalls++; throw new Error('Native updater must not run'); },
      async () => { externalCalls++; return notice; });
    expect(nativeCalls).toBe(0);
    expect(externalCalls).toBe(1);
    expect(update).toEqual(notice);
    expect(canInstallUpdate(policy(channel), update)).toBe(false);
  }
});

test('writable AppImages request the format-specific target and retain their installer', async () => {
  const installation = policy('appimage', true);
  const installer = { version: '0.2.7', body: 'AppImage notes' } as Update;
  let requestedTarget: string | undefined;
  const update = await checkAvailableUpdate(installation,
    async (options) => { requestedTarget = options?.target; return installer; },
    async () => { throw new Error('External check must not run'); });
  expect(requestedTarget).toBe('linux-x86_64-appimage');
  expect(update?.installer).toBe(installer);
  expect(canInstallUpdate(installation, update)).toBe(true);
  expect(canInstallUpdate(null, update)).toBe(false);
  expect(canInstallUpdate(policy('arch'), update)).toBe(false);
});

test('Windows and macOS keep their default updater target', async () => {
  let requestedTarget: string | undefined;
  const update = await checkAvailableUpdate(policy('internal', true),
    async (options) => { requestedTarget = options?.target; return null; });
  expect(requestedTarget).toBeUndefined();
  expect(update).toBeNull();
});

test('native checks give up instead of blocking the next ones', async () => {
  let timeout: number | undefined;
  await checkAvailableUpdate(policy('internal', true), async (options) => { timeout = options?.timeout; return null; });
  expect(timeout).toBe(CHECK_TIMEOUT_MS);
});

test('discarded updates release their native resource', async () => {
  let closed = 0;
  const installer = { version: '0.2.7', close: async () => { closed++; } } as Update;
  discardUpdate({ version: '0.2.7', installer });
  discardUpdate({ version: '0.2.7', download_url: null });
  discardUpdate(null);
  await Promise.resolve();
  expect(closed).toBe(1);
});

test('each Linux installation has matching update instructions', () => {
  for (const channel of ['arch', 'deb', 'rpm', 'flatpak', 'snap', 'portable', 'appimage'] as const) {
    expect(updateInstructionKey(policy(channel))).toBe(`update.${channel}Hint`);
  }
  expect(updateInstructionKey(policy('linux_package'))).toBe('update.packageHint');
  expect(updateInstructionKey(null)).toBe('update.packageHint');
});
