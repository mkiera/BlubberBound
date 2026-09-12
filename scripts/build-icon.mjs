import { cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const require = createRequire(import.meta.url);
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const hashFile = path => createHash('sha256').update(readFileSync(path)).digest('hex');

export function buildIcon(projectRoot = root) {
  const source = resolve(projectRoot, 'icon.png');
  const output = resolve(projectRoot, 'icon.ico');
  const cachePath = resolve(projectRoot, 'build/icon-source.json');
  const sourceHash = hashFile(source);
  const cliVersion = JSON.parse(readFileSync(require.resolve('@tauri-apps/cli/package.json'), 'utf8')).version;
  if (existsSync(cachePath) && existsSync(output)) {
    try {
      const cached = JSON.parse(readFileSync(cachePath, 'utf8'));
      if (cached.sourceHash === sourceHash && cached.outputHash === hashFile(output) && cached.cliVersion === cliVersion) return;
    } catch {}
  }
  const generated = resolve(projectRoot, 'build/generated-icons');
  mkdirSync(generated, { recursive: true });
  const result = spawnSync(process.execPath, [require.resolve('@tauri-apps/cli/tauri.js'), 'icon', source, '--output', generated], { cwd: projectRoot, encoding: 'utf8', windowsHide: true });
  if (result.status !== 0) throw new Error(result.stderr || result.stdout || 'Icon generation failed.');
  cpSync(resolve(generated, 'icon.ico'), output);
  writeFileSync(cachePath, JSON.stringify({ sourceHash, outputHash: hashFile(output), cliVersion }) + '\n');
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) buildIcon();
