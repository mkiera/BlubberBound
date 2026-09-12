import { cpSync, existsSync, lstatSync, mkdirSync, readdirSync, rmSync } from 'node:fs';
import { dirname, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { buildIcon } from './build-icon.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const frontendFiles = ['index.html', 'updates.html', 'notes.html', 'style.css', 'script.js', 'updates.js', 'notes.js', 'desktop.js', 'icon.ico', 'icon.png', 'fonts/OFL.txt'];

export function buildFrontend(projectRoot = root) {
  const destination = resolve(projectRoot, 'frontend-dist');
  if (relative(resolve(projectRoot), destination) !== 'frontend-dist') throw new Error('Invalid frontend output directory.');
  if (existsSync(destination) && lstatSync(destination).isSymbolicLink()) throw new Error('Frontend output cannot be a symbolic link.');
  const fonts = readdirSync(resolve(projectRoot, 'fonts'), { withFileTypes: true })
    .filter(entry => entry.isFile() && /\.(?:woff2?|ttf|otf)$/i.test(entry.name)).map(entry => `fonts/${entry.name}`);
  const files = [...frontendFiles, ...fonts];
  for (const name of files) {
    if (!lstatSync(resolve(projectRoot, name)).isFile()) throw new Error(`Frontend asset must be a regular file: ${name}`);
  }
  rmSync(destination, { recursive: true, force: true });
  for (const name of files) {
    const target = resolve(destination, name);
    mkdirSync(dirname(target), { recursive: true });
    cpSync(resolve(projectRoot, name), target);
  }
  return files;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  buildIcon(root);
  buildFrontend();
}
