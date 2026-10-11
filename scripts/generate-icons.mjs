import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const require = createRequire(import.meta.url);
const cli = require.resolve('@tauri-apps/cli/tauri.js');
const source = join(root, 'assets/brand/lownotes_logo.png');
const output = mkdtempSync(join(tmpdir(), 'lownotes-icons-'));

function generate() {
  const result = spawnSync(process.execPath, [cli, 'icon', source, '--output', output], {
    cwd: root,
    stdio: 'inherit',
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`Tauri icon generation failed (${result.status})`);
}

function update(generated, destination) {
  let bytes = readFileSync(join(output, generated));
  if (generated === 'icon.icns') {
    // The ICNS encoder emits its image chunks in random map order.
    // Sorting those chunks keeps the generated file reproducible.
    const chunks = [];
    for (let offset = 8; offset < bytes.length;) {
      const length = bytes.readUInt32BE(offset + 4);
      if (length < 8 || offset + length > bytes.length) throw new Error('Invalid generated ICNS chunk');
      chunks.push(bytes.subarray(offset, offset + length));
      offset += length;
    }
    chunks.sort((a, b) => Buffer.compare(a.subarray(0, 4), b.subarray(0, 4)));
    bytes = Buffer.concat([bytes.subarray(0, 8), ...chunks]);
  }
  const path = join(root, destination);
  // Keep unchanged files' timestamps so Cargo can reuse the compiled resources.
  if (!existsSync(path) || !readFileSync(path).equals(bytes)) writeFileSync(path, bytes);
}

try {
  generate();
  for (const name of ['32x32.png', '64x64.png', '128x128.png', '128x128@2x.png', 'icon.ico', 'icon.icns']) {
    update(name, `src-tauri/icons/${name}`);
  }
  update('64x64.png', 'static/favicon.png');
  console.log('App, installer, tray and favicon icons generated from assets/brand/lownotes_logo.png.');
} finally {
  rmSync(output, { recursive: true, force: true });
}
