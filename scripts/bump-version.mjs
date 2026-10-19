// Usage: bun run version:bump 0.3.6
// Updates every version-bearing file. Review the diff, then commit and tag.
import { SEMVER, readVersions, writeVersion } from './version-files.mjs';

const next = (process.argv[2] ?? '').replace(/^v/, '');
if (!SEMVER.test(next)) {
  console.error('Usage: bun run version:bump <major.minor.patch>');
  process.exit(1);
}
const current = readVersions()[0].version;
writeVersion(next);
console.log(`Version ${current} -> ${next}`);
for (const { path } of readVersions()) console.log(`  updated ${path}`);
console.log(`\nNext: git commit -am "chore: release v${next}" && git tag v${next}`);
