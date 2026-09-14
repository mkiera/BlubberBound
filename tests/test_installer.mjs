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
