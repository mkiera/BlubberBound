import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const installer = readFileSync(new URL('../installer.iss', import.meta.url), 'utf8');
const packaging = readFileSync(new URL('../build-package.ps1', import.meta.url), 'utf8');
const staging = readFileSync(new URL('../scripts/stage-payload.mjs', import.meta.url), 'utf8');

test('installer uses the product name for its executable, folder and shortcuts', () => {
  assert.match(installer, /DefaultDirName=\{localappdata\}\\Programs\\\{#StorageId\}/);
  assert.match(installer, /DefaultGroupName=\{#AppName\}/);
  assert.match(installer, /#define StorageId "BlubberBound"/);
  assert.match(installer, /#define Executable "BlubberBound.exe"/);
  assert.doesNotMatch(installer, /SealSqueeze|SealSuite/);
  assert.match(installer, /596690BB-D4AC-4C45-A35A-5C39B158156C/);
});

test('installer does not require accepting a license', () => {
  assert.doesNotMatch(installer, /^LicenseFile=/m);
});

test('installer retains payload rollback', () => {
  assert.match(installer, /RenameFile\(SavedPayload, LivePayload\)/);
  assert.match(installer, /Name: "\{app\}\\app"; BeforeInstall: BackupPayload/);
  assert.match(packaging, /test-installer\.ps1/);
});

test('packaging stages the renamed executable and emits no portable or checksum file', () => {
  assert.match(packaging, /target\\release\\blubberbound\.exe/);
  assert.match(staging, /target\/release\/blubberbound\.exe/);
  assert.doesNotMatch(packaging, /Compress-Archive|SHA256SUMS|Portable\.zip/);
  assert.doesNotMatch(staging, /normalizeZipTimes/);
});

test('launched programs do not hold the payload folder as their working directory', () => {
  const section = name => installer.split(`[${name}]`)[1].split(/^\[/m)[0];
  const entries = ['Icons', 'Run'].flatMap(name => section(name).split(/\r?\n/).filter(line => line.startsWith('Name:') || line.startsWith('Filename:')));
  assert.equal(entries.length, 3);
  for (const entry of entries) assert.match(entry, /; WorkingDir: "\{app\}"(;|$)/);
  const main = readFileSync(new URL('../src-tauri/src/main.rs', import.meta.url), 'utf8');
  const setup = main.slice(main.indexOf('.setup('));
  assert.match(setup, /let directory=std::path::absolute\(&directory\)\?;/);
  const change = setup.indexOf('std::env::set_current_dir(&directory)');
  assert.ok(setup.indexOf('std::path::absolute(&directory)') < setup.indexOf('Controller::new('));
  assert.ok(change > setup.indexOf('queue.add_paths(files)'), 'relative file arguments resolve before the change');
  assert.ok(change < setup.indexOf('updater.start()'));
});
