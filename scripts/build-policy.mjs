import { appendFileSync, readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

export function shouldBuildBranch(event, tags, changedFiles = null) {
  if (event === 'workflow_dispatch') return true;
  if (tags.some(tag => /^v\d+\.\d+\.\d+(?:-beta\.[1-9]\d*)?$/.test(tag))) return false;
  return changedFiles === null || changedFiles.some(file =>
    !file.endsWith('.md') && !file.startsWith('docs/') && !file.startsWith('screenshots/') &&
    !file.startsWith('.github/') && file !== 'scripts/build-policy.mjs' && file !== 'tests/test_build_policy.mjs');
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const tags = execFileSync('git', ['tag', '--points-at', 'HEAD'], { encoding: 'utf8' }).trim().split(/\r?\n/);
  const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
  const before = event.before;
  const changedFiles = before && !/^0+$/.test(before)
    ? execFileSync('git', ['diff', '--name-only', '-z', before, 'HEAD'], { encoding: 'utf8' }).split('\0').filter(Boolean)
    : null;
  const build = shouldBuildBranch(process.env.GITHUB_EVENT_NAME, tags, changedFiles);
  appendFileSync(process.env.GITHUB_OUTPUT, `build=${build}\n`);
  console.log(build ? 'Build branch installer.' : 'No app changes, or a release workflow builds this commit.');
}
