import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { appNotes, calculateAlpha, extractChangelog, normalizedVersion, numericVersion, parseVersion, validateReleaseTag } from '../scripts/versioning.mjs';
import { stampIdentity, validateTagPlacement } from '../scripts/build-identity.mjs';

test('semantic versions use official precedence and ignore build metadata', () => {
  const versions = ['1.0.0-alpha', '1.0.0-alpha.1', '1.0.0-alpha.beta', '1.0.0-beta', '1.0.0-beta.2', '1.0.0-beta.11', '1.0.0-rc.1', '1.0.0'];
  assert.deepEqual([...versions].reverse().sort((a, b) => parseVersion(a).compare(parseVersion(b))), versions);
  assert.equal(parseVersion('1.2.3+abc').compare(parseVersion('1.2.3+other')), 0);
  assert.equal(normalizedVersion('v1.2.3-beta.2+build.01'), '1.2.3-beta.2+build.01');
  assert.equal(numericVersion('1.2.3-beta.2+build'), '1.2.3.0');
  assert.throws(() => numericVersion('65536.0.0'));
});

test('semantic versions reject malformed strings', () => {
  for (const value of ['1', '1.2', '1.2.3.4', '01.2.3', '1.02.3', '1.2.03', '-1.2.3', '1.2.3-', '1.2.3+', '1.2.3-beta..2', '1.2.3-beta.01', '1.2.3-a_b', '1.2.3+a..b', 'vv1.2.3', ' 1.2.3', '1.2.3\n', '', null, '١.2.3']) {
    assert.throws(() => parseVersion(value), String(value));
  }
});

test('public tags allow only stable and numbered beta releases', () => {
  for (const tag of ['v0.0.0', 'v1.2.3', 'v1.2.3-beta.1', 'v1.2.3-beta.123']) assert.ok(validateReleaseTag(tag));
  for (const tag of ['1.2.3', 'v1.2', 'v01.2.3', 'v1.2.3b', 'v1.2.3-beta.0', 'v1.2.3-beta.01', 'v1.2.3-alpha.1', 'v1.2.3-rc.1', 'v1.2.3+build', 'v1.2.3-beta.1.alpha.2']) {
    assert.throws(() => validateReleaseTag(tag), tag);
  }
});

test('alpha calculations include stable tags outside branch history', () => {
  const cases = [
    [['0.1.0', null, 0, null, 17], '0.1.0-alpha.17'],
    [['1.4.0', 'v1.4.0-beta.1', 0, '1.3.2', 17], '1.4.0-beta.1'],
    [['1.4.0', 'v1.4.0-beta.1', 3, '1.3.2', 17], '1.4.0-beta.1.alpha.3'],
    [['1.4.0', 'v1.4.0', 2, '1.4.0', 17], '1.4.1-alpha.2'],
    [['1.5.0', 'v1.4.0', 2, '1.4.0', 17], '1.5.0-alpha.2'],
    [['1.4.0', 'v1.4.0-beta.1', 2, '1.4.0', 17], '1.4.1-alpha.2'],
    [['1.4.0', 'v1.4.0-beta.1', 2, '1.5.0', 17], '1.5.1-alpha.2'],
  ];
  for (const [args, expected] of cases) assert.equal(calculateAlpha(...args), expected);
  assert.throws(() => calculateAlpha('1.0.0', null, -1));
  assert.throws(() => calculateAlpha('1.0.0', null, 0, null, 0));
});

test('changelog extraction matches exact heading and requires content', () => {
  const text = '# Changelog\n\n## Unreleased\n\n## 1.2.3-beta.1 - 2026-01-01\n\n- Beta.\n\n## 1.2.3\n- Stable.\n\n## 1.2.2\n- Previous.\n';
  assert.equal(extractChangelog(text, 'v1.2.3'), '- Stable.');
  assert.equal(extractChangelog(text.replaceAll('\n', '\r\n'), '1.2.3-beta.1'), '- Beta.');
  assert.throws(() => extractChangelog('## 1.2.3-beta.1\n- Beta.\n', '1.2.3'));
  assert.throws(() => extractChangelog('## 1.2.3\n\n## 1.2.2\n- Old.\n', '1.2.3'));
  assert.equal(appNotes('- A.\r\n\r\n<!-- app-notes-end -->\nInstall this.'), '- A.');
  assert.equal(appNotes('Old notes without marker.'), 'Old notes without marker.');
});

test('tag placement requires remote heads and stable release merge', () => {
  const profile = { beta_branch: 'beta', stable_branch: 'main' };
  const answers = new Map([
    ['rev-parse v1.0.0^{commit}', 'merge'], ['rev-parse origin/main', 'merge'], ['rev-parse origin/beta', 'prepared'],
    ['rev-list --parents -n 1 merge', 'merge old prepared'], ['log -1 --format=%s merge', 'Merge beta for the 1.0.0 release'],
  ]);
  validateTagPlacement('v1.0.0', profile, args => answers.get(args.join(' ')));
  assert.throws(() => validateTagPlacement('v1.0.0-beta.1', profile, args => args[1].startsWith('v') ? 'wrong' : 'current'));
  answers.set('rev-list --parents -n 1 merge', 'merge old');
  assert.throws(() => validateTagPlacement('v1.0.0', profile, args => answers.get(args.join(' '))));
});

test('release stamping overrides build identity without editing intended core', () => {
  const root = mkdtempSync(join(tmpdir(), 'release-stamp-'));
  try {
    writeFileSync(join(root, 'version.txt'), '0.5.0\n');
    writeFileSync(join(root, 'CHANGELOG.md'), '## 1.0.0-beta.2\n- A change.\n');
    writeFileSync(join(root, 'app_profile.json'), JSON.stringify({ display_name: 'Test Product', installer_asset: 'Test-Setup.exe' }));
    const identity = stampIdentity({ tag: 'v1.0.0-beta.2' }, root, {}, () => 'abcdef123');
    assert.deepEqual(identity, { version: '1.0.0-beta.2', commit: 'abcdef123', branch: '', run_id: '' });
    assert.equal(readFileSync(join(root, 'version.txt'), 'utf8'), '0.5.0\n');
    assert.equal(appNotes(readFileSync(join(root, 'release-notes.md'), 'utf8')), '- A change.');
    const config = JSON.parse(readFileSync(join(root, 'src-tauri/build-config.json'), 'utf8'));
    assert.equal(config.version, identity.version);
    assert.equal(config.productName, 'Test Product');
  } finally { rmSync(root, { recursive: true, force: true }); }
});
