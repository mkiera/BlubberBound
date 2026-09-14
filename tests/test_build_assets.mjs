import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, utimesSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { tmpdir } from 'node:os';
import { buildFrontend, frontendFiles } from '../scripts/build-frontend.mjs';
import { normalizeZipTimes } from '../scripts/package-files.mjs';

test('frontend build copies only declared assets and removes stale output', () => {
  const root = mkdtempSync(join(tmpdir(), 'frontend-assets-'));
  try {
    for (const name of [...frontendFiles, 'fonts/typeface.woff2', 'fonts/source-notes.txt']) {
      mkdirSync(dirname(join(root, name)), { recursive: true });
      writeFileSync(join(root, name), name);
    }
    mkdirSync(join(root, 'frontend-dist'), { recursive: true });
    writeFileSync(join(root, 'frontend-dist/old-file.txt'), 'old');
    const files = buildFrontend(root);
    assert.ok(files.includes('icon.png'));
    assert.ok(files.includes('fonts/typeface.woff2'));
    assert.ok(existsSync(join(root, 'frontend-dist/fonts/OFL.txt')));
    assert.equal(existsSync(join(root, 'frontend-dist/fonts/source-notes.txt')), false);
    assert.equal(existsSync(join(root, 'frontend-dist/old-file.txt')), false);
    assert.equal(readFileSync(join(root, 'frontend-dist/icon.png'), 'utf8'), 'icon.png');
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('missing declared assets do not erase previous output', () => {
  const root = mkdtempSync(join(tmpdir(), 'frontend-missing-'));
  try {
    mkdirSync(join(root, 'fonts'));
    mkdirSync(join(root, 'frontend-dist'));
    writeFileSync(join(root, 'frontend-dist/index.html'), 'previous');
    assert.throws(() => buildFrontend(root));
    assert.equal(readFileSync(join(root, 'frontend-dist/index.html'), 'utf8'), 'previous');
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('copied license timestamps fit ZIP limits without changing contents', () => {
  const root = mkdtempSync(join(tmpdir(), 'package-times-'));
  try {
    const license = join(root, 'LICENSE');
    writeFileSync(license, 'License text');
    utimesSync(license, new Date(1000), new Date(1000));
    normalizeZipTimes(root, new Date('2026-01-01T12:00:00Z'));
    assert.equal(statSync(license).mtime.getUTCFullYear(), 2026);
    assert.equal(readFileSync(license, 'utf8'), 'License text');
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('Windows icon contains multiple image sizes', () => {
  const bytes = readFileSync(new URL('../icon.ico', import.meta.url));
  assert.equal(bytes.readUInt16LE(0), 0);
  assert.equal(bytes.readUInt16LE(2), 1);
  const count = bytes.readUInt16LE(4);
  assert.ok(count >= 4);
  const sizes = new Set(Array.from({ length: count }, (_, index) => bytes[6 + index * 16] || 256));
  assert.ok(sizes.has(16));
  assert.ok(sizes.has(32));
  assert.ok(sizes.has(256));
});
