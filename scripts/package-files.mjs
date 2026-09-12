import { readdirSync, statSync, utimesSync } from 'node:fs';
import { join } from 'node:path';

export function normalizeZipTimes(directory, fallback = new Date()) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isSymbolicLink()) throw new Error('Packaged files cannot be symbolic links.');
    if (entry.isDirectory()) normalizeZipTimes(path, fallback);
    const modified = statSync(path).mtime;
    if (modified.getFullYear() < 1980 || modified.getFullYear() > 2107) utimesSync(path, fallback, fallback);
  }
}
