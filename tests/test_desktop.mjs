import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

test('desktop bridge sends positional arguments and converts authorized media paths', async () => {
    const calls = [];
    const events = [];
    const listeners = new Map();
    const window = {
        __TAURI__: {
            core: {
                invoke: async (command, payload) => {
                    calls.push({command, payload});
                    return {preview: {source: 'original.mp4', path: 'sample.mp4'}};
                },
                convertFileSrc: path => `asset://${path}`,
            },
            event: {listen: (name, callback) => listeners.set(name, callback)},
        },
        dispatchEvent: event => events.push(event.type),
    };
    const context = vm.createContext({window, Event: class {constructor(type) {this.type = type;}}});
    vm.runInContext(fs.readFileSync(new URL('../desktop.js', import.meta.url), 'utf8'), context);
    const result = await window.desktop.api.start_preview('file-id', 2, 5);
    assert.equal(calls[0].command, 'desktop_command');
    assert.equal(calls[0].payload.method, 'start_preview');
    assert.deepEqual(Array.from(calls[0].payload.args), ['file-id', 2, 5]);
    assert.equal(result.preview.original_url, 'asset://original.mp4');
    assert.equal(result.preview.url, 'asset://sample.mp4');
    assert.deepEqual(events, ['desktopready']);
    await listeners.get('tauri://drag-drop')({payload: {paths: ['dropped.mp4']}});
    assert.equal(calls[1].payload.method, 'add_paths');
    assert.deepEqual(calls[1].payload.args[0], ['dropped.mp4']);
});

test('each desktop page loads the native bridge before application scripts', () => {
    for (const page of ['index.html', 'updates.html', 'notes.html']) {
        const html = fs.readFileSync(new URL(`../${page}`, import.meta.url), 'utf8');
        assert.ok(html.indexOf('src="desktop.js"') < html.indexOf('src="script.js"'));
    }
});
