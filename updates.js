'use strict';

const updateElement = id => document.getElementById(id);
const updateRows = new Map();
let updateState = null;
let updateReady = false;
let updateBusy = false;
let updatePolling = false;

function updateError(text) {
    updateElement('update-error').textContent = text || '';
    updateElement('update-error').hidden = !text;
}

function createUpdateRow(item) {
    const row = document.createElement('article');
    row.className = 'release-row';
    row.innerHTML = '<div class="release-heading"><div class="release-info"><h2></h2><span class="running-chip" hidden>Running now</span><p class="release-subtitle"></p></div><div class="release-actions"><button class="button secondary notes-toggle">Notes</button><button class="button primary install-release">Install</button></div></div><div class="release-notes" hidden><pre></pre><button class="text-button release-page">Open the release page</button></div><p class="downgrade-warning" role="alert" hidden></p>';
    const notes = row.querySelector('.release-notes');
    const toggle = row.querySelector('.notes-toggle');
    if (item.kind !== 'alpha') toggle.setAttribute('aria-expanded', 'false');
    toggle.addEventListener('click', () => {
        if (item.kind === 'alpha') {
            updateCommand('open_update_page', item.id);
            return;
        }
        notes.hidden = !notes.hidden;
        toggle.textContent = notes.hidden ? 'Notes' : 'Hide';
        toggle.setAttribute('aria-expanded', String(!notes.hidden));
        updateCommand('disarm_downgrade');
    });
    row.querySelector('.release-page').addEventListener('click', () => updateCommand('open_update_page', item.id));
    row.querySelector('.install-release').addEventListener('click', () => updateCommand('install_update', item.id));
    return row;
}

function renderUpdateState(state) {
    if (!state || !Array.isArray(state.rows)) return;
    updateState = state;
    document.title = `${state.branding?.display_name || 'SealSuite'} Updates`;
    const identity = state.identity || {};
    updateElement('running-version').textContent = `Running ${identity.version || 'unknown version'}${identity.branch && identity.commit ? `, commit ${identity.commit.slice(0, 7)}` : ''}`;
    let checked = state.checked_at;
    if (typeof checked === 'number') checked *= 1000;
    const date = checked ? new Date(checked) : null;
    updateElement('checked-at').textContent = date && !Number.isNaN(date.valueOf()) ? `Checked ${date.toLocaleString()}` : 'Not checked yet';
    const locked = updateBusy || Boolean(state.download?.active);
    updateElement('check-now').disabled = locked || state.checking;
    updateElement('check-now').textContent = state.checking ? 'Checking…' : 'Check now';
    document.querySelectorAll('[data-channel]').forEach(button => {
        button.setAttribute('aria-pressed', String(button.dataset.channel === state.channel));
        button.disabled = locked;
    });
    updateElement('automatic-updates').checked = Boolean(state.automatic);
    updateElement('automatic-updates').disabled = updateBusy;
    updateElement('update-message').textContent = state.download?.active ? state.download.status || 'Downloading installer…' : state.status || (state.checking ? 'Checking for updates…' : 'Choose a version to install.');
    updateElement('download-progress').hidden = !state.download?.active;
    updateElement('download-progress').value = Math.max(0, Math.min(100, state.download?.percent || 0));
    updateElement('empty-releases').hidden = state.rows.length > 0;
    updateElement('empty-releases').textContent = state.checking ? 'Loading versions…' : state.channel === 'alpha' ? 'No recent branch builds found. Artifacts expire after 30 days. Check again after a successful branch build.' : 'No installable releases found for this channel.';
    const ids = new Set(state.rows.map(row => row.id));
    for (const [id, row] of updateRows) {
        if (!ids.has(id)) {
            const focused = row.contains(document.activeElement);
            row.remove();
            updateRows.delete(id);
            if (focused) updateElement('check-now').focus();
        }
    }
    state.rows.forEach((item, index) => {
        if (!updateRows.has(item.id)) updateRows.set(item.id, createUpdateRow(item));
        const row = updateRows.get(item.id);
        const list = updateElement('release-rows');
        if (list.children[index] !== row) list.insertBefore(row, list.children[index] || null);
        row.querySelector('h2').textContent = item.title || item.version;
        row.querySelector('.running-chip').hidden = !item.running;
        row.querySelector('.release-subtitle').textContent = item.subtitle || '';
        row.querySelector('pre').textContent = item.notes || 'No notes were published for this release.';
        const toggle = row.querySelector('.notes-toggle');
        if (item.kind === 'alpha') toggle.textContent = 'Run';
        const armed = state.armed_id === item.id;
        const install = row.querySelector('.install-release');
        install.textContent = armed ? 'Confirm downgrade' : item.action || 'Install';
        install.disabled = locked;
        toggle.disabled = updateBusy;
        row.querySelector('.release-page').disabled = updateBusy;
        const warning = row.querySelector('.downgrade-warning');
        warning.hidden = !armed;
        warning.textContent = `Downgrade to ${item.version}? Application files move backward. Your files and settings remain. Press Confirm downgrade to continue.`;
    });
}

async function updateCommand(method, ...args) {
    if (!updateReady || updateBusy) return;
    updateBusy = true;
    updateError('');
    if (updateState) renderUpdateState(updateState);
    try {
        const result = await window.desktop.api[method](...args);
        if (result?.error) updateError(result.error);
        if (result?.rows) renderUpdateState(result);
    } catch (error) {
        updateError(error.message || String(error));
    } finally {
        updateBusy = false;
        if (updateState) renderUpdateState(updateState);
        pollUpdates();
    }
}

async function pollUpdates() {
    if (!updateReady || updateBusy || updatePolling) return;
    updatePolling = true;
    try {
        const state = await window.desktop.api.get_updates();
        if (!updateBusy) renderUpdateState(state);
    } catch (error) {
        updateError(error.message || String(error));
    } finally {
        updatePolling = false;
    }
}

function connectUpdates() {
    if (updateReady || !window.desktop?.api) return;
    updateReady = true;
    pollUpdates();
    window.setInterval(pollUpdates, 500);
}

updateElement('check-now').addEventListener('click', () => updateCommand('check_updates'));
updateElement('automatic-updates').addEventListener('change', event => updateCommand('set_automatic_updates', event.target.checked));
updateElement('close-updates').addEventListener('click', () => updateCommand('close_updates'));
document.querySelectorAll('[data-channel]').forEach(button => button.addEventListener('click', () => updateCommand('set_update_channel', button.dataset.channel)));
window.addEventListener('desktopready', connectUpdates);
connectUpdates();
