import test from 'node:test';
import assert from 'node:assert/strict';
import { shouldBuildBranch } from '../scripts/build-policy.mjs';

test('untagged branch commits build normally', () => {
  assert.equal(shouldBuildBranch('push', []), true);
  assert.equal(shouldBuildBranch('push', ['snapshot']), true);
});

test('release tags skip the duplicate branch installer', () => {
  assert.equal(shouldBuildBranch('push', ['v1.0.0-beta.1']), false);
  assert.equal(shouldBuildBranch('push', ['snapshot', 'v1.0.0']), false);
});

test('explicit manual builds remain available', () => {
  assert.equal(shouldBuildBranch('workflow_dispatch', ['v1.0.0-beta.1']), true);
});

test('documentation and workflow changes skip the app build', () => {
  assert.equal(shouldBuildBranch('push', [], ['README.md', '.github/workflows/build-test.yml', 'scripts/build-policy.mjs', 'tests/test_build_policy.mjs']), false);
  assert.equal(shouldBuildBranch('push', [], ['docs/usage.txt']), false);
});

test('mixed changes and packaging inputs still build', () => {
  for (const file of ['src-tauri/src/main.rs', 'icon.png', 'build-package.ps1', 'scripts/stage-payload.mjs', 'LICENSE']) {
    assert.equal(shouldBuildBranch('push', [], ['README.md', file]), true);
  }
});
