// Every file that carries the application version. Keep this list in one place
// so bumping and checking can never drift apart.
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const read = (path) => readFileSync(root + path, 'utf8');
const write = (path, text) => writeFileSync(root + path, text);

const CARGO_PACKAGE = /(\[package\][^[]*?\nversion\s*=\s*")([^"]+)(")/;
const CARGO_LOCK = /(\[\[package\]\]\r?\nname = "lownotes"\r?\nversion = ")([^"]+)(")/;
const JSON_VERSION = /("version"\s*:\s*")([^"]+)(")/;

export const VERSION_FILES = [
  { path: 'package.json', pattern: JSON_VERSION },
  { path: 'src-tauri/tauri.conf.json', pattern: JSON_VERSION },
  { path: 'src-tauri/Cargo.toml', pattern: CARGO_PACKAGE },
  { path: 'src-tauri/Cargo.lock', pattern: CARGO_LOCK },
];

export function readVersions() {
  return VERSION_FILES.map(({ path, pattern }) => {
    const match = read(path).match(pattern);
    if (!match) throw new Error(`No version found in ${path}`);
    return { path, version: match[2] };
  });
}

export function writeVersion(version) {
  for (const { path, pattern } of VERSION_FILES) {
    const text = read(path);
    if (!pattern.test(text)) throw new Error(`No version found in ${path}`);
    write(path, text.replace(pattern, `$1${version}$3`));
  }
}

export const SEMVER = /^\d+\.\d+\.\d+$/;
