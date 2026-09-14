'use strict';

if (window.__TAURI__) {
    const {invoke, convertFileSrc} = window.__TAURI__.core;
    window.desktop = {api: new Proxy({}, {
        get: (_, method) => async (...args) => {
            const result = await invoke('desktop_command', {method, args});
            if (result?.preview) {
                const preview = result.preview;
                preview.original_url = preview.source ? convertFileSrc(preview.source) : '';
                preview.url = preview.path ? convertFileSrc(preview.path) : '';
            }
            return result;
        },
    })};
    window.__TAURI__.event.listen('tauri://drag-drop', async event => {
        try {
            await window.desktop.api.add_paths(event.payload.paths);
        } catch (error) {
            const banner = document.getElementById('app-error');
            if (banner) { banner.textContent = String(error); banner.hidden = false; }
        }
    });
    window.dispatchEvent(new Event('desktopready'));
}
