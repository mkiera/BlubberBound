'use strict';

function formatBytes(value) {
    if (value === null || value === undefined || !Number.isFinite(Number(value))) return 'Unknown size';
    const bytes = Math.max(0, Number(value));
    if (bytes < 1000) return `${Math.round(bytes)} B`;
    const units = ['kB', 'MB', 'GB', 'TB'];
    const exponent = Math.min(Math.floor(Math.log10(bytes) / 3), 4);
    return `${(bytes / 1000 ** exponent).toLocaleString('en-US', {maximumFractionDigits: 1})} ${units[exponent - 1]}`;
}

function savingsLabel(original, output) {
    if (!(original > 0) || !Number.isFinite(output)) return '';
    const percent = Math.round((1 - output / original) * 100);
    return percent > 0 ? `${percent}% smaller` : percent < 0 ? `${-percent}% larger` : 'Same size';
}

const advancedDefaults = {advanced_enabled: false, rate_control: 'target', video_bitrate_kbps: 2500, audio_bitrate_kbps: 128, crf: 23, scale_percent: 100, output_width: 0, output_height: 0, fps: 0, preset: 'veryfast', audio_channels: 0, audio_sample_rate: 0, mute_audio: false, image_quality: 90, image_lossless: false, strip_metadata: true};

function validateSettings(settings) {
    const target = Number(settings.target_mb);
    if (!Number.isFinite(target) || target < 0.1 || target > 100000) {
        throw new Error('Choose a size limit from 0.1 to 100,000 MB.');
    }
    const result = {...advancedDefaults, ...settings, target_mb: target};
    const bounds = {video_bitrate_kbps: [12, 100000], audio_bitrate_kbps: [8, 320], crf: [0, 51], scale_percent: [10, 200], output_width: [0, 7680], output_height: [0, 7680], fps: [0, 120], image_quality: [1, 100]};
    for (const [key, [min, max]] of Object.entries(bounds)) {
        const number = Number(result[key]);
        if (result[key] === '' || !Number.isFinite(number) || number < min || number > max || (!['fps', 'scale_percent'].includes(key) && !Number.isInteger(number))) throw new Error(`Choose ${key.replaceAll('_', ' ')} from ${min} to ${max}.`);
        result[key] = number;
    }
    if (result.output_width === 1 || result.output_height === 1) throw new Error('Dimensions must be zero or at least two pixels.');
    for (const [key, choices] of Object.entries({rate_control: ['target', 'bitrate', 'quality'], preset: ['ultrafast', 'superfast', 'veryfast', 'faster', 'fast', 'medium', 'slow', 'slower', 'veryslow'], audio_channels: [0, 1, 2], audio_sample_rate: [0, 22050, 32000, 44100, 48000]})) {
        if (typeof choices[0] === 'number') result[key] = Number(result[key]);
        if (!choices.includes(result[key])) throw new Error(`Choose a supported ${key.replaceAll('_', ' ')}.`);
    }
    return result;
}

function previewRange(job, start, duration) {
    if (job.kind === 'image') return {start: 0, duration: 5};
    const position = Number(start);
    const length = Number(duration);
    if (start === '' || !Number.isFinite(position) || position < 0 || !(job.duration > position)) throw new Error('Choose a preview start inside the file.');
    if (duration === '' || !Number.isFinite(length) || length < 1 || length > 15) throw new Error('Choose a sample length from 1 to 15 seconds.');
    return {start: position, duration: length};
}

function previewSettingsChanged(previous, current) {
    const effective = settings => {
        const result = {...advancedDefaults, ...settings};
        if (!result.advanced_enabled) Object.assign(result, advancedDefaults);
        if (result.rate_control === 'quality') result.encoder = 'software';
        return result;
    };
    previous = effective(previous);
    current = effective(current);
    const keys = ['target_mb', 'video_format', 'audio_format', 'image_format', 'max_height', 'encoder', ...Object.keys(advancedDefaults)];
    return keys.some(key => String(previous?.[key] ?? advancedDefaults[key]) !== String(current?.[key] ?? advancedDefaults[key]));
}

function canReplaceOutput(job, settings) {
    const format = settings?.[`${job?.kind}_format`];
    const extension = format === 'jpeg' ? 'jpg' : format;
    return Boolean(job?.output && extension && job.output.toLowerCase().endsWith(`.${extension}`));
}

if (typeof module !== 'undefined') module.exports = {formatBytes, savingsLabel, validateSettings, advancedDefaults, previewRange, previewSettingsChanged, canReplaceOutput};

if (typeof document !== 'undefined' && new URLSearchParams(window.location.search).get('demo') === '1' && !window.desktop) {
    const previewUpdates = {
        branding: {display_name: 'SealSuite preview'},
        identity: {version: '1.0.0', commit: '', branch: ''}, channel: 'stable', automatic: true,
        checked_at: Date.now() / 1000, checking: false, status: 'Demo preview. No files are downloaded or installed.',
        rows: [
            {id: 'new', version: '1.1.0', title: '1.1.0', subtitle: 'Stable / 12 September 2026 / 105 MB', notes: '- Faster batch exports.\n- Better handling of long filenames.\n\n<script>This remains plain text.</script>', action: 'Update', kind: 'release'},
            {id: 'current', version: '1.0.0', title: '1.0.0', subtitle: 'Stable / 11 September 2026 / 102 MB', notes: '- Compress video, audio, and images.\n- Originals stay untouched.', action: 'Reinstall', running: true, kind: 'release'},
            {id: 'old', version: '0.9.0', title: '0.9.0', subtitle: 'Stable / 10 September 2026 / 101 MB', notes: '', action: 'Downgrade', kind: 'release'},
        ], offer: null, download: {active: false, percent: 0, status: ''}, armed_id: null,
        whats_new: {title: 'What’s new in 1.0.0', count: 2, sections: [{version: '1.0.0', notes: '- Compress video, audio, and images.\n- Originals stay untouched.'}]},
    };
    const releasePreviewRows = [...previewUpdates.rows];
    const previewState = {
        branding: previewUpdates.branding, version: '1.0.0', tools: {ffmpeg: true, ffprobe: true},
        settings: {target_mb: 25, video_format: 'mp4', audio_format: 'mp3', image_format: 'webp', max_height: 0, encoder: 'auto', output_dir: ''},
        jobs: [
            {id: 'video', name: 'A quiet morning at the beach.mp4', source: 'Demo files / beach.mp4', kind: 'video', duration: 120, original_size: 86500000, status: 'completed', percent: 100, stage: 'Finished', output: 'Demo files / beach-squeezed.mp4', output_size: 24100000},
            {id: 'audio', name: 'Ocean ambience.wav', source: 'Demo files / ocean.wav', kind: 'audio', duration: 90, original_size: 52000000, status: 'pending', percent: 0, stage: ''},
            {id: 'image', name: 'An exceptionally long holiday photograph filename with spaces and punctuation.jpg', source: 'Demo files / photograph.jpg', kind: 'image', original_size: 8300000, status: 'failed', percent: 0, stage: '', error: 'Example error: the requested limit is too small. Increase the limit and retry.'},
        ], running: false, flipperclipper: true, updates: previewUpdates,
    };
    const clone = value => JSON.parse(JSON.stringify(value));
    const demoResult = async () => ({ok: true, error: 'Demo preview only. Open the desktop app to process files.'});
    window.desktop = {api: {
        get_state: async () => clone(previewState), get_updates: async () => clone(previewUpdates),
        update_settings: async settings => { Object.assign(previewState.settings, settings); return clone(previewState); },
        add_files: demoResult, add_folder: demoResult, choose_output: demoResult,
        use_source_folder: async () => { previewState.settings.output_dir = ''; return clone(previewState); },
        start_queue: demoResult, cancel_current: demoResult, stop_queue: demoResult,
        start_preview: demoResult, cancel_preview: demoResult, open_preview: demoResult, preview_source: demoResult, rerun_job: demoResult,
        remove_job: async id => { previewState.jobs = previewState.jobs.filter(job => job.id !== id); return clone(previewState); },
        retry_job: async id => { const job = previewState.jobs.find(job => job.id === id); if (job) { job.status = 'pending'; job.error = ''; } return clone(previewState); },
        clear_finished: async () => { previewState.jobs = previewState.jobs.filter(job => job.status === 'pending'); return clone(previewState); },
        open_output: demoResult, show_output: demoResult, open_in_clipper: demoResult,
        open_updates: async () => { window.location.href = 'updates.html?demo=1'; },
        read_whats_new: async () => { window.location.href = 'notes.html?demo=1'; },
        dismiss_whats_new: async () => { previewUpdates.whats_new = null; return clone(previewState); },
        dismiss_update: async () => { previewUpdates.offer = null; return clone(previewState); },
        check_updates: async () => clone(previewUpdates),
        set_update_channel: async channel => { previewUpdates.channel = channel; previewUpdates.armed_id = null; previewUpdates.rows = channel === 'alpha' ? [{id: 'branch', title: 'feature/batch #17', subtitle: 'commit a1b2c3d, 12 September 2026', kind: 'alpha', action: 'Install'}] : [...releasePreviewRows]; return clone(previewUpdates); },
        set_automatic_updates: async enabled => { previewUpdates.automatic = enabled; return clone(previewUpdates); },
        disarm_downgrade: async () => { previewUpdates.armed_id = null; return clone(previewUpdates); },
        install_update: async id => { if (id === 'old' && previewUpdates.armed_id !== id) previewUpdates.armed_id = id; else { previewUpdates.armed_id = null; previewUpdates.status = 'Demo preview only. No installer was launched.'; } return clone(previewUpdates); },
        open_update_page: demoResult,
        close_updates: async () => { window.location.href = 'index.html?demo=1'; },
        close_whats_new: async () => { window.location.href = 'index.html?demo=1'; },
    }};
    const preview = document.createElement('div');
    preview.className = 'preview-label';
    preview.textContent = 'DEMO PREVIEW / Example data only';
    document.body.prepend(preview);
}

if (typeof document !== 'undefined' && document.getElementById('jobs')) {
    const $ = id => document.getElementById(id);
    const terminal = new Set(['completed', 'failed', 'cancelled']);
    const active = new Set(['probing', 'running']);
    const rows = new Map();
    let state = null;
    let ready = false;
    let busy = 0;
    let commands = Promise.resolve();
    let polling = false;
    let settingsDirty = false;
    let completionKey = '';
    let productName = 'SealSuite media compressor';
    let previewJobId = null;
    let renderedPreviewKey = '';

    function message(text) {
        $('notice-text').textContent = text;
        $('notice').hidden = !text;
        for (const [dialog, error] of [['advanced-dialog', 'advanced-error'], ['preview-dialog', 'preview-error'], ['rerun-dialog', 'rerun-error']]) {
            if ($(dialog).open) { $(error).textContent = text; $(error).hidden = !text; }
        }
    }

    function readSettings() {
        const advanced = {};
        for (const key of Object.keys(advancedDefaults)) {
            const input = document.querySelector(`[name="${key}"]`);
            advanced[key] = input.type === 'checkbox' ? input.checked : input.value;
        }
        return validateSettings({
            ...advanced,
            target_mb: $('target-mb').value,
            video_format: $('video-format').value,
            audio_format: $('audio-format').value,
            image_format: $('image-format').value,
            max_height: Number($('max-height').value),
            encoder: $('encoder').value,
        });
    }

    function renderSettings(settings) {
        if (settingsDirty || $('settings-form').contains(document.activeElement) || $('advanced-form').contains(document.activeElement)) return;
        for (const [key, value] of Object.entries({...advancedDefaults, ...settings})) {
            const input = document.querySelector(`[name="${key}"]`);
            if (input?.type === 'checkbox') input.checked = Boolean(value);
            else if (input) input.value = value ?? (key === 'max_height' ? 0 : '');
        }
        renderPresets();
        renderAdvanced();
    }

    function renderAdvanced() {
        const enabled = $('advanced-enabled').checked;
        const mode = $('rate-control').value;
        const sizeMode = !enabled || mode === 'target';
        $('advanced-controls').classList.toggle('inactive-controls', !enabled);
        $('advanced-controls').querySelectorAll('input, select').forEach(input => { input.disabled = !enabled; });
        $('video-bitrate').disabled = !enabled || mode !== 'bitrate';
        $('crf').disabled = !enabled || mode !== 'quality';
        $('target-mb').disabled = !sizeMode;
        document.querySelectorAll('[data-target]').forEach(button => { button.disabled = !sizeMode; });
        $('encoder').disabled = enabled && mode === 'quality';
        $('target-help').textContent = sizeMode ? 'Smaller limits trade detail for size. Files that cannot meet your limit will report an error.' : 'Advanced mode controls quality or bitrate. Output size is not limited.';
        $('rate-help').textContent = ({target: 'Fit size limit adjusts quality to meet the size limit. Audio bitrate is an upper limit.', bitrate: 'Uses your video and audio bitrates. Output size is not limited. Image quality is set separately.', quality: 'Uses software video quality and your audio bitrate. Output size is not limited. Image quality is set separately.'})[mode];
        $('open-advanced').textContent = enabled ? 'Advanced: on' : 'Advanced';
        $('scale-slider').value = $('scale-percent').value;
    }

    function renderPresets() {
        document.querySelectorAll('[data-target]').forEach(button => {
            button.setAttribute('aria-pressed', String(Number(button.dataset.target) === Number($('target-mb').value)));
        });
    }

    function showPreview(id) {
        const job = state?.jobs.find(item => item.id === id);
        if (!job || state.running || state.preview?.status === 'running') return;
        previewJobId = id;
        renderedPreviewKey = '';
        $('preview-name').textContent = job.name;
        $('preview-time-fields').hidden = job.kind === 'image';
        const lastStart = Math.max(0, Math.ceil((Number(job.duration) - 0.1) * 10) / 10);
        for (const input of [$('preview-start'), $('preview-start-slider')]) { input.max = String(lastStart); input.value = '0'; }
        $('preview-duration').value = '5';
        $('preview-error').hidden = true;
        $('preview-dialog').showModal();
        renderPreview(state.preview);
    }

    function mediaElement(container, kind, url, start = 0, duration = null) {
        container.replaceChildren();
        if (!url) { container.textContent = 'Use Open original to view this file in your player.'; return; }
        const media = document.createElement(kind === 'image' ? 'img' : kind === 'audio' ? 'audio' : 'video');
        if (kind === 'image') media.alt = 'Quality comparison';
        else {
            media.controls = true;
            media.preload = 'metadata';
            if (start > 0) media.addEventListener('loadedmetadata', () => { media.currentTime = start; }, {once: true});
            if (duration) media.addEventListener('timeupdate', () => { if (media.currentTime >= start + duration) { media.pause(); media.currentTime = start; } });
        }
        media.addEventListener('error', () => {
            if (!container.contains(media)) return;
            container.replaceChildren();
            container.textContent = 'This format cannot play here. Open it in your system player.';
        }, {once: true});
        media.src = url;
        container.append(media);
    }

    function renderPreview(preview) {
        if (!$('preview-dialog').open) return;
        const job = state?.jobs.find(item => item.id === previewJobId);
        const matching = preview?.job_id === previewJobId;
        const running = preview?.status === 'running';
        const completed = matching && preview.status === 'completed';
        $('preview-fields').disabled = !ready || busy > 0 || state.running || running || !job;
        $('cancel-preview').hidden = !running;
        $('cancel-preview').disabled = busy > 0;
        $('open-preview').hidden = !completed;
        $('open-preview').disabled = busy > 0 || running;
        $('preview-progress').hidden = !matching || !running;
        $('preview-progress').value = Number(preview?.percent) || 0;
        $('preview-status').textContent = matching ? preview.stage || ({running: 'Generating preview…', completed: 'Preview ready', failed: 'Preview failed', cancelled: 'Preview cancelled'})[preview.status] || '' : 'Choose a sample and generate a preview.';
        if (matching && preview.error) { $('preview-error').textContent = preview.error; $('preview-error').hidden = false; }
        $('preview-comparison').hidden = !completed;
        $('preview-result').textContent = completed ? `${formatBytes(preview.size)}${preview.kind !== 'image' ? ` / ${Number(preview.duration_seconds).toFixed(1)} seconds` : ''}. ${preview.quality_note || 'Temporary preview. Your queued file has not been compressed.'}` : '';
        let changed = false;
        if (completed) {
            try { changed = previewSettingsChanged(preview.settings, readSettings()); } catch { changed = true; }
            if (job?.kind !== 'image') changed ||= Number($('preview-start').value) !== Number(preview.start_seconds) || Math.abs(Math.min(Number($('preview-duration').value), Number(job?.duration) - Number(preview.start_seconds)) - Number(preview.duration_seconds)) > 0.05;
        }
        $('preview-stale').hidden = !completed || !changed;
        const key = completed ? JSON.stringify([preview.path, preview.url, preview.original_url, preview.start_seconds]) : '';
        if (key !== renderedPreviewKey) {
            renderedPreviewKey = key;
            $('original-media').replaceChildren();
            $('preview-media').replaceChildren();
            if (completed) {
                mediaElement($('original-media'), preview.kind, preview.original_url || preview.source_url, preview.start_seconds || 0, preview.duration_seconds);
                const unchanged = preview.preserves_original && preview.original_url;
                mediaElement($('preview-media'), preview.kind, unchanged || preview.url,
                    unchanged ? preview.start_seconds || 0 : 0, unchanged ? preview.duration_seconds : null);
            }
        }
    }

    function showRerun(id) {
        const jobs = state?.jobs.filter(job => job.status === 'completed' && job.output) || [];
        if (!jobs.length || state.running || state.preview?.status === 'running') return;
        $('rerun-file').replaceChildren(...jobs.map(job => {
            const option = document.createElement('option');
            option.value = job.id;
            option.textContent = job.name;
            return option;
        }));
        if (id) $('rerun-file').value = id;
        renderRerunPath();
        $('rerun-error').hidden = true;
        $('rerun-dialog').showModal();
    }

    function renderRerunPath() {
        const job = state?.jobs.find(item => item.id === $('rerun-file').value);
        $('rerun-path').textContent = job?.output || '';
        let settings;
        try { settings = readSettings(); } catch { settings = state?.settings; }
        const compatible = canReplaceOutput(job, settings);
        $('rerun-format-help').hidden = compatible;
        $('rerun-replace').disabled = !compatible || !ready || busy > 0 || state?.running || state?.preview?.status === 'running';
    }

    function jobRow(job) {
        const row = document.createElement('li');
        row.className = 'job';
        row.innerHTML = '<div class="job-main"><span class="kind-icon" aria-hidden="true"></span><div class="job-info"><h3 class="job-name"></h3><p class="job-meta"></p></div><span class="job-status"></span></div><progress max="100"></progress><p class="job-stage"></p><p class="job-error"></p><p class="job-output"></p><div class="job-actions"></div>';
        const actions = row.querySelector('.job-actions');
        const buttons = {};
        for (const [action, label] of [['quality_preview', 'Preview quality'], ['squeeze_again', 'Squeeze again'], ['open_output', 'Open file'], ['show_output', 'Show in folder'], ['open_in_clipper', 'Pass to FlipperClipper'], ['retry_job', 'Retry'], ['remove_job', 'Remove']]) {
            const button = document.createElement('button');
            button.className = 'text-button';
            button.textContent = label;
            button.setAttribute('aria-label', `${label}: ${job.name}`);
            button.addEventListener('click', () => action === 'quality_preview' ? showPreview(job.id) : action === 'squeeze_again' ? showRerun(job.id) : command(action, job.id));
            actions.append(button);
            buttons[action] = button;
        }
        return {row, buttons};
    }

    function renderJob(job, entry) {
        const {row, buttons} = entry;
        const query = selector => row.querySelector(selector);
        row.dataset.status = job.status;
        query('.kind-icon').textContent = ({video: 'VID', audio: 'AUD', image: 'IMG'})[job.kind] || 'FILE';
        query('.job-name').textContent = job.name;
        query('.job-name').title = job.source;
        const done = job.status === 'completed';
        const savings = done ? savingsLabel(job.original_size, job.output_size) : '';
        query('.job-meta').textContent = `${formatBytes(job.original_size)}${done ? ` → ${formatBytes(job.output_size)}${savings ? ` / ${savings}` : ''}` : ` / ${job.kind || 'file'}`}`;
        const percent = Math.max(0, Math.min(100, Number(job.percent) || 0));
        query('.job-status').textContent = ({pending: 'Queued', probing: 'Reading file', running: `${Math.round(percent)}%`, completed: 'Complete', failed: 'Failed', cancelled: 'Cancelled'})[job.status] || job.status;
        const progress = query('progress');
        progress.hidden = !active.has(job.status);
        progress.value = percent;
        progress.setAttribute('aria-label', `Compression progress: ${job.name}`);
        query('.job-stage').textContent = job.stage || '';
        query('.job-stage').hidden = !job.stage;
        query('.job-error').textContent = job.error || '';
        query('.job-error').hidden = !job.error;
        query('.job-output').textContent = done && job.output ? job.output : '';
        query('.job-output').hidden = !done || !job.output;
        buttons.open_output.hidden = !done || !job.output;
        buttons.show_output.hidden = !done || !job.output;
        buttons.open_in_clipper.hidden = !done || !job.output || job.kind !== 'video' || !state.flipperclipper;
        buttons.retry_job.hidden = !['failed', 'cancelled'].includes(job.status);
        buttons.squeeze_again.hidden = !done || !job.output;
        buttons.remove_job.hidden = active.has(job.status);
        for (const button of Object.values(buttons)) button.disabled = busy > 0 || !ready;
        const processing = state.running || state.preview?.status === 'running';
        buttons.quality_preview.hidden = active.has(job.status) || !['video', 'audio', 'image'].includes(job.kind);
        buttons.quality_preview.disabled ||= processing || !state.tools?.ffmpeg || !state.tools?.ffprobe;
        buttons.remove_job.disabled ||= processing;
        buttons.retry_job.disabled ||= processing;
        buttons.squeeze_again.disabled ||= processing;
    }

    function render(next) {
        if (!next || !Array.isArray(next.jobs)) return;
        state = next;
        productName = next.branding?.display_name || productName;
        $('product-name').textContent = productName;
        document.title = productName;
        if (next.error) message(next.error);
        $('version').textContent = next.version ? `v${next.version}` : '';
        renderUpdates(next.updates);
        renderSettings(next.settings);
        const output = next.settings?.output_dir;
        $('output-dir').textContent = output || 'Each file’s original folder';
        $('output-dir').title = output || 'Each file’s original folder';
        $('use-source').hidden = !output;
        const available = ready && !busy;
        const previewRunning = next.preview?.status === 'running';
        const processing = next.running || previewRunning;
        const hasPending = next.jobs.some(job => job.status === 'pending');
        const hasCompleted = next.jobs.some(job => job.status === 'completed' && job.output);
        const hasActive = next.jobs.some(job => active.has(job.status));
        const completed = next.jobs.filter(job => job.status === 'completed').length;
        const failed = next.jobs.filter(job => job.status === 'failed').length;
        const count = next.jobs.length;
        document.querySelector('.queue-panel').classList.toggle('has-jobs', count > 0);
        $('queue-count').textContent = `${count} ${count === 1 ? 'file' : 'files'}`;
        $('empty-queue').hidden = count > 0;
        for (const id of ['add-files', 'add-folder']) $(id).disabled = !available;
        for (const id of ['choose-output', 'use-source', 'open-advanced']) $(id).disabled = !available || processing;
        $('settings-fields').disabled = !available || processing;
        $('advanced-fields').disabled = !available || processing;
        $('save-advanced').disabled = !available || processing;
        $('clear-finished').disabled = !available || processing || !next.jobs.some(job => terminal.has(job.status));
        $('start-queue').disabled = !available || (!hasPending && !hasCompleted) || processing || !next.tools?.ffmpeg || !next.tools?.ffprobe;
        $('start-queue').firstChild.textContent = !hasPending && hasCompleted ? 'Squeeze again ' : 'Start squeezing ';
        $('start-queue').hidden = next.running;
        $('running-actions').hidden = !next.running;
        $('cancel-current').disabled = !available || !hasActive;
        $('stop-queue').disabled = !available || !next.running;
        $('start-hint').textContent = previewRunning ? 'A quality preview is being generated.' : next.running ? 'Settings stay fixed until this batch stops.' : hasPending ? 'Your settings apply to all queued files.' : hasCompleted ? 'Choose a completed file to compress with current settings.' : 'Add a file or retry an unfinished item.';
        for (const id of ['rerun-copy', 'rerun-replace', 'rerun-file']) $(id).disabled = !available || processing;
        if ($('rerun-dialog').open) renderRerunPath();
        const toolsMissing = !next.tools?.ffmpeg || !next.tools?.ffprobe;
        $('tools-status').textContent = toolsMissing ? 'Media tools missing. Install FFmpeg and FFprobe, then restart.' : '';
        $('tools-status').hidden = !toolsMissing;
        const statusText = next.running ? `Squeezing / ${completed} of ${count} complete` : count ? `${completed} complete${failed ? ` / ${failed} failed` : ''}${hasPending ? ' / Ready to start' : ''}` : 'Your queue is ready for files.';
        if ($('queue-status').textContent !== statusText) $('queue-status').textContent = statusText;
        const ids = new Set(next.jobs.map(job => job.id));
        for (const [id, entry] of rows) {
            if (!ids.has(id)) {
                const focused = entry.row.contains(document.activeElement);
                entry.row.remove();
                rows.delete(id);
                if (focused) $('add-files').focus();
            }
        }
        for (const job of next.jobs) {
            if (!rows.has(job.id)) {
                const entry = jobRow(job);
                rows.set(job.id, entry);
                $('jobs').append(entry.row);
            }
            renderJob(job, rows.get(job.id));
        }
        renderPreview(next.preview);
        const key = next.jobs.filter(job => terminal.has(job.status)).map(job => `${job.id}:${job.status}`).join(',');
        if (key !== completionKey) {
            completionKey = key;
            document.title = next.running ? `${productName} / ${completed} complete` : productName;
        }
    }

    function renderUpdates(updates) {
        $('open-updates').disabled = !ready || busy > 0;
        if (!updates) return;
        const identity = updates.identity || {};
        if (identity.version) $('version').textContent = `${identity.version}${identity.branch && identity.commit ? `, ${identity.commit.slice(0, 7)}` : ''}`;
        const downloading = Boolean(updates.download?.active);
        const offer = updates.offer;
        $('update-banner').hidden = !offer && !downloading;
        $('update-title').textContent = offer ? `${productName} ${offer.version} is available.` : 'Downloading update';
        $('update-status').textContent = updates.download?.status || '';
        $('update-progress').hidden = !downloading;
        $('update-progress').value = updates.download?.percent || 0;
        $('install-offer').disabled = !offer || downloading || busy > 0;
        $('dismiss-offer').disabled = downloading || busy > 0;
        const notes = updates.whats_new;
        $('notes-banner').hidden = !notes;
        if (notes) {
            $('notes-title').textContent = notes.title;
            $('notes-count').textContent = notes.count === 1 ? 'One change.' : `${notes.count} changes.`;
        }
    }

    function command(method, ...args) {
        if (!ready) return Promise.resolve();
        busy++;
        if (state) render(state);
        commands = commands.catch(() => {}).then(async () => {
            try {
                const result = await window.desktop.api[method](...args);
                if (result?.error) message(result.error);
                if (result?.jobs) render(result);
                return result;
            } catch (error) {
                message(error.message || String(error));
            } finally {
                busy--;
                if (state) render(state);
            }
        });
        return commands;
    }

    async function saveSettings() {
        if (!settingsDirty) return true;
        let settings;
        try {
            settings = readSettings();
        } catch (error) {
            message(error.message);
            return false;
        }
        const result = await command('update_settings', settings);
        if (result && !result.error) settingsDirty = false;
        return Boolean(result && !result.error);
    }

    async function poll() {
        if (!ready || busy || polling) return;
        polling = true;
        try {
            const next = await window.desktop.api.get_state();
            if (!busy) render(next);
            $('connection').hidden = true;
        } catch (error) {
            $('connection').hidden = false;
            $('connection').textContent = `Connection interrupted: ${error.message || error}. Retrying…`;
        } finally {
            polling = false;
        }
    }

    function connect() {
        if (ready || !window.desktop?.api) return;
        ready = true;
        poll();
        window.setInterval(poll, 500);
    }

    $('dismiss-notice').addEventListener('click', () => message(''));
    $('open-settings').addEventListener('click', () => {
        $('settings-title').scrollIntoView({block: 'nearest'});
        $('settings-title').focus();
    });
    $('open-updates').addEventListener('click', () => command('open_updates'));
    $('update-details').addEventListener('click', () => command('open_updates'));
    $('install-offer').addEventListener('click', () => { if (state?.updates?.offer) command('install_update', state.updates.offer.id); });
    $('dismiss-offer').addEventListener('click', () => command('dismiss_update'));
    $('read-notes').addEventListener('click', () => command('read_whats_new'));
    $('dismiss-notes').addEventListener('click', () => command('dismiss_whats_new'));
    $('add-files').addEventListener('click', () => command('add_files'));
    $('add-folder').addEventListener('click', () => command('add_folder'));
    $('choose-output').addEventListener('click', () => command('choose_output'));
    $('use-source').addEventListener('click', () => command('use_source_folder'));
    $('clear-finished').addEventListener('click', () => command('clear_finished'));
    $('cancel-current').addEventListener('click', () => command('cancel_current'));
    $('stop-queue').addEventListener('click', () => command('stop_queue'));
    $('start-queue').addEventListener('click', async () => {
        if (!state?.jobs.some(job => job.status === 'pending')) { showRerun(); return; }
        if (await saveSettings()) {
            message('');
            await command('start_queue');
        }
    });
    $('settings-form').addEventListener('submit', event => event.preventDefault());
    $('settings-form').addEventListener('input', () => { settingsDirty = true; renderPresets(); });
    $('settings-form').addEventListener('change', () => { settingsDirty = true; saveSettings(); });
    document.querySelectorAll('[data-target]').forEach(button => button.addEventListener('click', () => {
        $('target-mb').value = button.dataset.target;
        settingsDirty = true;
        renderPresets();
        saveSettings();
    }));
    $('open-advanced').addEventListener('click', () => { $('advanced-error').hidden = true; $('advanced-dialog').showModal(); });
    $('advanced-form').addEventListener('input', event => {
        if (event.target.id === 'scale-slider') $('scale-percent').value = event.target.value;
        settingsDirty = true;
        renderAdvanced();
    });
    $('advanced-form').addEventListener('submit', async event => { event.preventDefault(); if (await saveSettings()) $('advanced-dialog').close(); });
    $('close-advanced').addEventListener('click', async () => { if (await saveSettings()) $('advanced-dialog').close(); });
    $('advanced-dialog').addEventListener('cancel', async event => { event.preventDefault(); if (await saveSettings()) $('advanced-dialog').close(); });
    $('preview-start-slider').addEventListener('input', event => { $('preview-start').value = event.target.value; });
    $('preview-start').addEventListener('input', event => { $('preview-start-slider').value = event.target.value; });
    $('preview-form').addEventListener('submit', async event => {
        event.preventDefault();
        try {
            const job = state?.jobs.find(item => item.id === previewJobId);
            if (!job) throw new Error('This file is no longer in the queue.');
            const range = previewRange(job, $('preview-start').value, $('preview-duration').value);
            if (await saveSettings()) { message(''); await command('start_preview', previewJobId, range.start, range.duration); }
        } catch (error) { message(error.message); }
    });
    $('preview-original').addEventListener('click', () => command('preview_source', previewJobId));
    $('cancel-preview').addEventListener('click', () => command('cancel_preview'));
    $('open-preview').addEventListener('click', () => command('open_preview'));
    function closePreview() {
        for (const media of $('preview-dialog').querySelectorAll('video, audio')) media.pause();
        $('preview-dialog').close();
        if (state?.preview?.status === 'running') command('cancel_preview');
    }
    $('close-preview').addEventListener('click', closePreview);
    $('preview-dialog').addEventListener('cancel', event => { event.preventDefault(); closePreview(); });
    $('rerun-file').addEventListener('change', renderRerunPath);
    $('cancel-rerun').addEventListener('click', () => $('rerun-dialog').close());
    for (const [id, mode] of [['rerun-copy', 'copy'], ['rerun-replace', 'replace']]) $(id).addEventListener('click', async () => {
        if (await saveSettings()) {
            const result = await command('rerun_job', $('rerun-file').value, mode);
            if (result && !result.error) $('rerun-dialog').close();
        }
    });
    let dragDepth = 0;
    $('drop-zone').addEventListener('dragenter', event => {
        event.preventDefault();
        dragDepth++;
        $('drop-zone').classList.add('drag-over');
    });
    $('drop-zone').addEventListener('dragleave', () => {
        dragDepth = Math.max(0, dragDepth - 1);
        if (!dragDepth) $('drop-zone').classList.remove('drag-over');
    });
    document.addEventListener('dragover', event => event.preventDefault());
    document.addEventListener('drop', event => {
        event.preventDefault();
        dragDepth = 0;
        $('drop-zone').classList.remove('drag-over');
    });
    document.addEventListener('keydown', event => {
        if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'o' && !event.repeat) {
            event.preventDefault();
            if (!$('add-files').disabled) command('add_files');
        }
    });
    window.addEventListener('desktopready', connect);
    connect();
    window.setTimeout(() => {
        if (!ready) {
            $('connection').textContent = 'Open the installed app or run.bat to use the desktop interface. A regular browser cannot access the local media tools.';
            $('tools-status').textContent = 'Desktop app required';
            $('tools-status').hidden = false;
        }
    }, 2500);
}
