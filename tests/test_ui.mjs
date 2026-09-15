import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const context = vm.createContext({module: {exports: {}}});
vm.runInContext(fs.readFileSync(new URL('../script.js', import.meta.url), 'utf8'), context);
const {formatBytes, savingsLabel, validateSettings, advancedDefaults, previewRange, previewSettingsChanged, canReplaceOutput} = context.module.exports;



test('formats sizes without misleading zero values', () => {
    assert.equal(formatBytes(0), '0 B');
    assert.equal(formatBytes(null), 'Unknown size');
    assert.equal(formatBytes(12500000), '12.5 MB');
});

test('progress refreshes quickly during compression and slows when idle', async () => {
    const source = fs.readFileSync(new URL('../script.js', import.meta.url), 'utf8');
    const connect = source.slice(source.indexOf('    function connect()'), source.indexOf("    $('dismiss-notice')"));
    const scheduled = [];
    const state = {running: true, preview: {status: 'idle'}};
    const sandbox = {ready: false, state, poll: async () => {}, window: {desktop: {api: {}}, setTimeout: (callback, delay) => scheduled.push({callback, delay})}};
    vm.runInNewContext(`${connect}\nconnect();`, sandbox);
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(scheduled[0].delay, 100);
    state.running = false;
    state.preview.status = 'running';
    await scheduled.shift().callback();
    assert.equal(scheduled[0].delay, 100);
    state.preview.status = 'ready';
    await scheduled.shift().callback();
    assert.equal(scheduled[0].delay, 500);
    assert.equal(scheduled.length, 1);
});

test('reports savings and larger output truthfully', () => {
    assert.equal(savingsLabel(1000, 750), '25% smaller');
    assert.equal(savingsLabel(1000, 1100), '10% larger');
    assert.equal(savingsLabel(1000, 1000), 'Same size');
});

test('rejects invalid size limits before calling the desktop bridge', () => {
    for (const value of ['', 0, -1, 'bad', Infinity]) {
        assert.throws(() => validateSettings({target_mb: value}), /size/i);
    }
    assert.equal(validateSettings({target_mb: '25'}).target_mb, 25);
});

test('auto quality mode is distinct from a size limit and invalid modes are rejected', () => {
    const html = fs.readFileSync(new URL('../index.html', import.meta.url), 'utf8');
    assert.match(html, /name="compression_mode"[^>]*>[^<]*<option value="limit"[^>]*>[^<]*<\/option><option value="auto"/);
    assert.equal(validateSettings({target_mb: 25, compression_mode: 'auto'}).compression_mode, 'auto');
    assert.throws(() => validateSettings({target_mb: 25, compression_mode: 'unknown'}), /compression mode/i);
    const source = fs.readFileSync(new URL('../script.js', import.meta.url), 'utf8');
    assert.match(source, /\$\('target-mb'\)\.disabled = auto/);
    assert.match(source, /buttons\.quality_preview\.hidden = \$\('compression-mode'\)\.value === 'auto'/);
    assert.match(source, /\$\('basic-formats'\)\.hidden = auto/);
    assert.match(source, /\$\('encoder'\)\.hidden = auto/);
    assert.match(html, /id="auto-formats-info"[^>]*>Auto uses MKV video, FLAC audio, and lossless WebP images/);
});

test('every static DOM lookup resolves in its window', () => {
    for (const [htmlFile, scriptFile, pattern] of [
        ['index.html', 'script.js', /\$\('([^']+)'\)/g],
        ['updates.html', 'updates.js', /updateElement\('([^']+)'\)/g],
        ['notes.html', 'notes.js', /getElementById\('([^']+)'\)/g],
    ]) {
        const html = fs.readFileSync(new URL(`../${htmlFile}`, import.meta.url), 'utf8');
        const script = fs.readFileSync(new URL(`../${scriptFile}`, import.meta.url), 'utf8');
        const ids = new Set([...html.matchAll(/\bid="([^"]+)"/g)].map(match => match[1]));
        for (const [, id] of script.matchAll(pattern)) assert.ok(ids.has(id), `${htmlFile} is missing #${id}`);
    }
});

test('updates precedes accessible settings and channels retain stable order', () => {
    const html = fs.readFileSync(new URL('../index.html', import.meta.url), 'utf8');
    assert.ok(html.indexOf('id="open-updates"') < html.indexOf('id="open-settings"'));
    assert.match(html, /id="open-settings"[^>]+aria-label="Settings"/);
    const updates = fs.readFileSync(new URL('../updates.html', import.meta.url), 'utf8');
    assert.deepEqual([...updates.matchAll(/data-channel="([^"]+)"/g)].map(match => match[1]), ['stable', 'prerelease', 'alpha']);
});

test('header shows the app version without a suite connection indicator', () => {
    const html = fs.readFileSync(new URL('../index.html', import.meta.url), 'utf8');
    const headerTools = html.slice(html.indexOf('class="header-tools"'), html.indexOf('</header>'));
    assert.match(headerTools, /id="version"/);
    assert.doesNotMatch(headerTools, /SealSuite|status-dot|suite-label/);
});

test('release notes render as text and assets remain local', () => {
    for (const scriptFile of ['updates.js', 'notes.js']) {
        const source = fs.readFileSync(new URL(`../${scriptFile}`, import.meta.url), 'utf8');
        assert.match(source, /textContent = (?:item\.notes|section\.notes)/);
        assert.doesNotMatch(source, /innerHTML\s*=\s*(?:item|section|state)\./);
    }
    for (const htmlFile of ['index.html', 'updates.html', 'notes.html']) {
        const html = fs.readFileSync(new URL(`../${htmlFile}`, import.meta.url), 'utf8');
        assert.doesNotMatch(html, /(?:src|href)="https?:/);
    }
});

test('settings remain expanded and the desktop window reserves room for them', () => {
    const html = fs.readFileSync(new URL('../index.html', import.meta.url), 'utf8');
    const optionsStart = html.indexOf('class="settings-options"');
    const formEnd = html.indexOf('</form>', optionsStart);
    assert.ok(html.indexOf('id="target-mb"') < optionsStart);
    assert.ok(html.indexOf('id="encoder"') > optionsStart);
    assert.ok(html.indexOf('id="start-queue"') > formEnd);
    assert.doesNotMatch(html, /class="settings-options"[^>]+tabindex/);
    const css = fs.readFileSync(new URL('../style.css', import.meta.url), 'utf8');
    assert.doesNotMatch(css, /\.settings-options\s*\{[^}]*overflow\s*:\s*(auto|scroll|hidden)/);
    const app = JSON.parse(fs.readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
    assert.equal(app.app.windows[0].minWidth, 1000);
    assert.equal(app.app.windows[0].minHeight, 1080);
});

test('main window uses plain headings and reserves tool status for errors', () => {
    const html = fs.readFileSync(new URL('../index.html', import.meta.url), 'utf8');
    const script = fs.readFileSync(new URL('../script.js', import.meta.url), 'utf8');
    assert.match(html, /id="queue-title"[^>]*>Files<\/h2>/);
    assert.match(html, /id="settings-title"[^>]*>Compression settings<\/h2>/);
    assert.match(html, /id="tools-status"[^>]* hidden><\/footer>/);
    assert.match(script, /\$\('tools-status'\)\.hidden = !toolsMissing/);
    for (const text of [
        'Drop a little weight.',
        'Everything stays on your computer.',
        'Originals stay untouched. Existing filenames get a numbered copy.',
        'FFmpeg ready / processing stays local',
        'Local files. Smaller copies. Same seal family.',
        'A LITTLE LIGHTER. READY TO SHARE.',
    ]) assert.ok(!html.includes(text) && !script.includes(text), text);
});

test('header uses the supplied product icon', () => {
    const html = fs.readFileSync(new URL('../index.html', import.meta.url), 'utf8');
    assert.match(html, /class="seal"[^>]*><img src="icon\.png" alt="">/);
    assert.doesNotMatch(html, /🦭/);
});

test('advanced controls match every persisted advanced setting', () => {
    const html = fs.readFileSync(new URL('../index.html', import.meta.url), 'utf8');
    for (const key of Object.keys(advancedDefaults)) assert.ok(html.includes(`name="${key}"`), key);
    assert.match(html, /<dialog id="advanced-dialog"/);
    assert.match(html, /<dialog id="preview-dialog"/);
    assert.match(html, /id="preview-stale"/);
    assert.doesNotMatch(html, /ffmpeg arguments|command line/i);
});

test('advanced settings convert numeric form values even while disabled', () => {
    const settings = validateSettings({target_mb: '25', audio_channels: '2', audio_sample_rate: '48000', scale_percent: '75.5', fps: '29.97'});
    assert.equal(settings.audio_channels, 2);
    assert.equal(settings.audio_sample_rate, 48000);
    assert.equal(settings.scale_percent, 75.5);
    assert.equal(settings.fps, 29.97);
    assert.equal(settings.advanced_enabled, false);
    for (const invalid of [{crf: 52}, {output_width: 1}, {fps: Infinity}, {video_bitrate_kbps: 11}, {audio_bitrate_kbps: 321}, {preset: 'custom'}, {rate_control: 'raw'}, {image_quality: 0}]) assert.throws(() => validateSettings({target_mb: 25, advanced_enabled: true, ...invalid}));
});

test('preview seeks remain inside the source and sample length stays bounded', () => {
    const video = {kind: 'video', duration: 90};
    assert.equal(previewRange(video, '20', '5').start, 20);
    assert.equal(previewRange(video, '89.5', '15').duration, 15);
    for (const [start, duration] of [[90, 5], [-1, 5], ['', 5], [0, 0], [0, 16], [Infinity, 5]]) assert.throws(() => previewRange(video, start, duration));
    assert.equal(previewRange({kind: 'image'}, '', '').start, 0);
});

test('stale preview checks use effective settings and ignore output folder', () => {
    const base = {target_mb: 25, encoder: 'auto', advanced_enabled: false};
    assert.equal(previewSettingsChanged(base, {...base, output_dir: 'elsewhere'}), false);
    assert.equal(previewSettingsChanged(base, {...base, crf: 45}), false);
    assert.equal(previewSettingsChanged(base, {...base, target_mb: 10}), true);
    assert.equal(previewSettingsChanged({...base, advanced_enabled: true, rate_control: 'quality', encoder: 'software'}, {...base, advanced_enabled: true, rate_control: 'quality'}), false);
    assert.equal(previewSettingsChanged({...base, advanced_enabled: true}, {...base, advanced_enabled: true, scale_percent: 50}), true);
    assert.equal(previewSettingsChanged(base, {...base, compression_mode: 'auto'}), true);
});

test('replacing a previous output requires the same output extension', () => {
    assert.equal(canReplaceOutput({kind: 'video', output: 'C:/output/clip.MP4'}, {video_format: 'mp4'}), true);
    assert.equal(canReplaceOutput({kind: 'video', output: 'C:/output/clip.mp4'}, {video_format: 'webm'}), false);
    assert.equal(canReplaceOutput({kind: 'image', output: 'photo.jpg'}, {image_format: 'jpeg'}), true);
    assert.equal(canReplaceOutput({kind: 'image'}, {image_format: 'jpeg'}), false);
    assert.equal(canReplaceOutput({kind: 'video', output: 'C:/output/clip.mkv'}, {compression_mode: 'auto', video_format: 'mp4'}), true);
    assert.equal(canReplaceOutput({kind: 'video', output: 'C:/output/clip.mp4'}, {compression_mode: 'auto', video_format: 'mp4'}), false);
    assert.equal(canReplaceOutput({kind: 'video', output: 'C:/source.mp4', source: 'C:/source.mp4'}, {video_format: 'mp4'}), false);
    const html = fs.readFileSync(new URL('../index.html', import.meta.url), 'utf8');
    for (const label of ['Another copy', 'Replace previous output', 'Cancel']) assert.ok(html.includes(label));
    const script = fs.readFileSync(new URL('../script.js', import.meta.url), 'utf8');
    assert.match(script, /!hasPending && hasCompleted \? 'Compress again '/);
    assert.match(script, /command\('rerun_job', \$\('rerun-file'\)\.value, mode\)/);
});
