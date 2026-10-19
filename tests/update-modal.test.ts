import { expect, test } from 'bun:test';
import { compile } from 'svelte/compiler';
import { render } from 'svelte/server';
import type { Update } from '@tauri-apps/plugin-updater';
import { locale, translate } from '../src/lib/i18n';
import type { UpdateChannel, UpdatePolicy } from '../src/lib/types';

Bun.plugin({
  name: 'svelte-server-tests',
  setup(build) {
    build.onLoad({ filter: /\.svelte$/ }, async ({ path }) => ({
      contents: compile(await Bun.file(path).text(), { filename: path, generate: 'server' }).js.code,
      loader: 'js',
    }));
  },
});

const { default: UpdateModal } = await import('../src/lib/components/UpdateModal.svelte');

test('Linux package dialogs explain their update method and offer no in-app installer', () => {
  locale.set('pt-BR');
  for (const channel of ['arch', 'deb', 'rpm', 'flatpak', 'snap', 'portable', 'linux_package', 'appimage'] as const) {
    const policy: UpdatePolicy = { channel, can_install: false, updater_target: null };
    const output = render(UpdateModal, { props: { isOpen: true, policy,
      update: { version: '0.2.7', body: 'Notes', download_url: null } } });
    expect(output.body).not.toContain('Atualizar agora');
    const hint = channel === 'linux_package' ? 'package' : channel;
    expect(output.body).toContain(translate('pt-BR', `update.${hint}Hint`));
    expect(output.body).toContain('Ver release');
  }
});

test('a matching native package download is available without an install button', () => {
  locale.set('pt-BR');
  const output = render(UpdateModal, { props: { isOpen: true,
    policy: { channel: 'deb', can_install: false, updater_target: null },
    update: { version: '0.2.7', download_url: 'https://github.com/LowBloat/LowNotes/releases/download/v0.2.7/LowNotes-0.2.7-linux-x64.deb' } } });
  expect(output.body).toContain('Baixar atualização');
  expect(output.body).not.toContain('Atualizar agora');
});

test('supported internal updates retain their install button', () => {
  locale.set('pt-BR');
  for (const channel of ['internal', 'appimage'] as UpdateChannel[]) {
    const installer = { version: '0.2.7' } as Update;
    const output = render(UpdateModal, { props: { isOpen: true,
      policy: { channel, can_install: true, updater_target: null },
      update: { version: '0.2.7', installer } } });
    expect(output.body).toContain('Atualizar agora');
    expect(output.body).not.toContain('Baixar atualização');
  }
});

test('release notes render as Markdown without raw HTML', () => {
  locale.set('en-US');
  const body = [
    '## ✨ Features',
    '',
    '- **Remove vaults from the list** — see [the release](https://github.com/LowBloat/LowNotes/releases).',
    '- `code` and <script>alert(1)</script>',
    '',
    '![shot](https://example.com/shot.png)',
  ].join('\n');
  const output = render(UpdateModal, { props: { isOpen: true,
    policy: { channel: 'internal', can_install: true, updater_target: null },
    update: { version: '0.4.0', body, installer: { version: '0.4.0' } as Update } } });
  expect(output.body).toContain('<h2>✨ Features</h2>');
  expect(output.body).toContain('<strong>Remove vaults from the list</strong>');
  expect(output.body).toContain('<a href="https://github.com/LowBloat/LowNotes/releases">the release</a>');
  expect(output.body).toContain('<code>code</code>');
  expect(output.body).not.toContain('<script>');
  expect(output.body).not.toContain('**Remove');
  expect(output.body).not.toContain('<img');
  expect(output.body).toContain('data-remote-src="https://example.com/shot.png"');
});
