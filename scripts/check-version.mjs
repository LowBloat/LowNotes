// Usage: bun scripts/check-version.mjs [vX.Y.Z]
// Fails when the version-bearing files disagree, or differ from the given tag.
import { readVersions } from './version-files.mjs';

const versions = readVersions();
const distinct = [...new Set(versions.map((entry) => entry.version))];
for (const { path, version } of versions) console.log(`${version.padEnd(10)} ${path}`);
if (distinct.length !== 1) {
  console.error('Version files disagree. Run `bun run version:bump <version>`.');
  process.exit(1);
}
const tag = process.argv[2];
if (tag && tag !== `v${distinct[0]}`) {
  console.error(`Tag ${tag} does not match version ${distinct[0]}.`);
  process.exit(1);
}
if (process.env.GITHUB_OUTPUT) {
  const { appendFileSync } = await import('node:fs');
  appendFileSync(process.env.GITHUB_OUTPUT, `version=${distinct[0]}\n`);
}
