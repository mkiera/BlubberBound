import semver from 'semver';

export function parseVersion(value) {
  if (typeof value !== 'string' || !/^(?:v)?(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.test(value)) {
    throw new Error(`Invalid semantic version: ${value}`);
  }
  return new semver.SemVer(value.startsWith('v') ? value.slice(1) : value);
}

export function validateReleaseTag(tag) {
  if (typeof tag !== 'string' || !/^v(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-beta\.[1-9][0-9]*)?$/.test(tag)) {
    throw new Error('Release tags must be vMAJOR.MINOR.PATCH or vMAJOR.MINOR.PATCH-beta.N, with N at least 1.');
  }
  return parseVersion(tag);
}

export function normalizedVersion(value) {
  const parsed = parseVersion(value);
  return parsed.version + (parsed.build.length ? `+${parsed.build.join('.')}` : '');
}

export function coreVersion(value) {
  const parsed = parseVersion(value);
  return `${parsed.major}.${parsed.minor}.${parsed.patch}`;
}

export function numericVersion(value) {
  const parsed = parseVersion(value);
  const numbers = [parsed.major, parsed.minor, parsed.patch, 0];
  if (numbers.some(number => number > 65535)) throw new Error('Windows version components must be 0 through 65535.');
  return numbers.join('.');
}

export function calculateAlpha(aimedCore, nearestTag = null, distance = 0, latestStable = null, runNumber = 1) {
  const aimed = coreVersion(aimedCore);
  if (!Number.isSafeInteger(distance) || distance < 0 || !Number.isSafeInteger(runNumber) || runNumber < 1) {
    throw new Error('Distance must be nonnegative and run number positive.');
  }
  if (!nearestTag) return `${aimed}-alpha.${runNumber}`;
  const nearest = parseVersion(nearestTag);
  if (distance === 0) return normalizedVersion(nearestTag);
  let base;
  if (semver.gt(aimed, coreVersion(nearestTag))) base = aimed;
  else if (nearest.prerelease.length) {
    if (latestStable && semver.gte(coreVersion(latestStable), coreVersion(nearestTag))) {
      const floor = semver.inc(coreVersion(latestStable), 'patch');
      base = semver.gt(aimed, floor) ? aimed : floor;
    } else return `${nearest.version}.alpha.${distance}`;
  } else base = semver.inc(coreVersion(nearestTag), 'patch');
  return `${base}-alpha.${distance}`;
}

export function extractChangelog(text, version) {
  const normalized = normalizedVersion(version);
  const escaped = normalized.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = new RegExp(`^##[^\\S\\r\\n]+${escaped}(?=\\s|$)[^\\r\\n]*`, 'm').exec(text);
  if (!match) throw new Error(`Missing changelog section for ${normalized}`);
  const following = text.slice(match.index + match[0].length);
  const boundary = /^## /m.exec(following);
  const body = (boundary ? following.slice(0, boundary.index) : following).trim();
  if (!body) throw new Error(`Empty changelog section for ${normalized}`);
  return body;
}

export function appNotes(body) {
  return String(body || '').replace(/\r\n?/g, '\n').split('<!-- app-notes-end -->', 1)[0].trim();
}
