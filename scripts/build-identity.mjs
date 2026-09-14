import { appendFileSync, readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';
import { parseArgs } from 'node:util';
import semver from 'semver';
import { calculateAlpha, extractChangelog, normalizedVersion, numericVersion, validateReleaseTag } from './versioning.mjs';

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

export function git(args, required = true, root = projectRoot) {
  const result = spawnSync('git', args, { cwd: root, encoding: 'utf8', windowsHide: true });
  if (result.status !== 0 && required) throw new Error(result.stderr?.trim() || 'Git command failed.');
  return result.status === 0 ? result.stdout.trim() : '';
}

export function validateTagPlacement(tag, profile, readGit = args => git(args)) {
  const version = validateReleaseTag(tag);
  const target = readGit(['rev-parse', `${tag}^{commit}`]);
  const branch = version.prerelease.length ? profile.beta_branch : profile.stable_branch;
  if (target !== readGit(['rev-parse', `origin/${branch}`])) throw new Error(`Release tag must point at the current remote ${branch} head.`);
  if (!version.prerelease.length) {
    const parents = readGit(['rev-list', '--parents', '-n', '1', target]).split(/\s+/);
    if (parents.length !== 3) throw new Error('Stable release must be a two-parent merge from beta.');
    if (parents[2] !== readGit(['rev-parse', `origin/${profile.beta_branch}`])) throw new Error('Stable release second parent must be the prepared remote beta head.');
    if (readGit(['log', '-1', '--format=%s', target]) !== `Merge ${profile.beta_branch} for the ${version.version} release`) throw new Error('Stable release merge subject does not match its version.');
  }
}

export function stampIdentity(args = {}, root = projectRoot, environment = process.env, readGit = (values, required = true) => git(values, required, root)) {
  const profile = JSON.parse(readFileSync(resolve(root, 'app_profile.json'), 'utf8'));
  const aimed = readFileSync(resolve(root, 'version.txt'), 'utf8').trim();
  let version = args.version || aimed;
  const identity = { version: '', commit: readGit(['rev-parse', 'HEAD'], false), branch: '', run_id: '' };
  if (args.tag) {
    version = validateReleaseTag(args.tag).version;
    if (args['validate-placement']) validateTagPlacement(args.tag, profile, readGit);
    const body = extractChangelog(readFileSync(resolve(root, 'CHANGELOG.md'), 'utf8'), version);
    const installation = `\n\n<!-- app-notes-end -->\n### Installation\nRun ${profile.installer_asset} to install for your Windows account. FFmpeg and FFprobe are included. Microsoft Edge WebView2 Runtime is required.\n\n### Already have it?\nRun the installer over your existing installation. Your folder, shortcuts, queue, and settings are kept.\n\n### Uninstalling\nUninstall through Windows Settings. Files you compressed and your saved queue and settings remain on disk.\n`;
    writeFileSync(resolve(root, 'release-notes.md'), body + installation);
  } else if (args.alpha) {
    const describe = readGit(['describe', '--tags', '--long', '--match', 'v[0-9]*'], false);
    const match = /^(.+)-(\d+)-g[0-9a-f]+$/.exec(describe);
    const stableTags = readGit(['tag', '--list', 'v*']).split(/\r?\n/).flatMap(tag => {
      try { const parsed = validateReleaseTag(tag); return parsed.prerelease.length ? [] : [parsed.version]; } catch { return []; }
    }).sort(semver.rcompare);
    version = calculateAlpha(aimed, match?.[1], match ? Number(match[2]) : 0, stableTags[0], Number(args['run-number'] || environment.GITHUB_RUN_NUMBER || 1));
    identity.branch = args.branch ?? environment.GITHUB_REF_NAME ?? '';
    identity.run_id = String(args['run-id'] ?? environment.GITHUB_RUN_ID ?? '');
  }
  identity.version = normalizedVersion(version);
  numericVersion(identity.version);
  writeFileSync(resolve(root, 'build_info.json'), JSON.stringify(identity, null, 2) + '\n');
  mkdirSync(resolve(root, 'src-tauri'), { recursive: true });
  writeFileSync(resolve(root, 'src-tauri/build-config.json'), JSON.stringify({ productName: profile.display_name, version: identity.version }, null, 2) + '\n');
  if (environment.GITHUB_OUTPUT) appendFileSync(environment.GITHUB_OUTPUT, `version=${identity.version}\nprerelease=${semver.prerelease(identity.version) !== null}\ninstaller=${profile.installer_asset}\n`);
  return identity;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const { values } = parseArgs({ options: { version: { type: 'string' }, tag: { type: 'string' }, alpha: { type: 'boolean' }, 'validate-placement': { type: 'boolean' }, branch: { type: 'string' }, 'run-id': { type: 'string' }, 'run-number': { type: 'string' } } });
    if ([values.version, values.tag, values.alpha].filter(Boolean).length > 1) throw new Error('Choose only one version source.');
    console.log(JSON.stringify(stampIdentity(values)));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
