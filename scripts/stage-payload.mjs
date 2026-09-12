import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { normalizeZipTimes } from './package-files.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const destination = process.argv[2];
if (!destination) throw new Error('A payload directory is required.');
const profile = JSON.parse(readFileSync(join(root, 'app_profile.json'), 'utf8'));
const identity = JSON.parse(readFileSync(join(root, 'build_info.json'), 'utf8'));
mkdirSync(destination, { recursive: true });
cpSync(join(root, 'src-tauri/target/release/sealsqueeze.exe'), join(destination, profile.executable));
for (const file of ['app_profile.json', 'build_info.json', 'LICENSE', 'THIRD_PARTY_NOTICES.md']) cpSync(join(root, file), join(destination, file));
writeFileSync(join(destination, 'version.txt'), identity.version + '\n');
mkdirSync(join(destination, 'tools'), { recursive: true });
for (const file of ['ffmpeg.exe', 'ffprobe.exe', 'FFMPEG-LICENSE.txt', 'FFMPEG-README.txt']) cpSync(join(root, 'tools', file), join(destination, 'tools', file));
mkdirSync(join(destination, 'fonts'), { recursive: true });
cpSync(join(root, 'fonts/OFL.txt'), join(destination, 'fonts/OFL.txt'));
const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--manifest-path', join(root, 'src-tauri/Cargo.toml'), '--locked', '--format-version', '1'], { cwd: root, encoding: 'utf8', windowsHide: true, maxBuffer: 32 * 1024 * 1024 }));
const notices = [];
for (const dependency of metadata.packages.filter(value => value.source).sort((a, b) => a.name.localeCompare(b.name))) {
  notices.push(`${dependency.name} ${dependency.version}\nLicense: ${dependency.license || 'See included license file'}\nSource: ${dependency.repository || `https://crates.io/crates/${dependency.name}/${dependency.version}`}\n`);
  const directory = dirname(dependency.manifest_path);
  const licenseRoot = join(destination, 'licenses', `${dependency.name}-${dependency.version}`);
  for (const file of readdirSync(directory).filter(value => /^(?:licen[sc]e|copying|copyright|notice)(?:[-.]|$)/i.test(value))) {
    mkdirSync(licenseRoot, { recursive: true });
    cpSync(join(directory, file), join(licenseRoot, file), { recursive: true });
  }
  if (dependency.license_file && existsSync(join(directory, dependency.license_file))) {
    mkdirSync(licenseRoot, { recursive: true });
    cpSync(join(directory, dependency.license_file), join(licenseRoot, 'LICENSE'), { recursive: true });
  }
}
writeFileSync(join(destination, 'THIRD_PARTY_LICENSES.txt'), notices.join('\n'));
normalizeZipTimes(destination);
