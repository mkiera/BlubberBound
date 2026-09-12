'use strict';

let notesReady = false;
async function loadNotes() {
    if (notesReady || !window.desktop?.api) return;
    notesReady = true;
    try {
        const state = await window.desktop.api.get_updates();
        const productName = state.branding?.display_name || 'SealSuite';
        document.title = `What's new in ${productName}`;
        document.getElementById('notes-product').textContent = productName.toUpperCase();
        const notes = state.whats_new;
        document.getElementById('notes-heading').textContent = notes?.title || 'What’s new';
        const container = document.getElementById('notes-sections');
        for (const section of notes?.sections || []) {
            const article = document.createElement('section');
            if (notes.sections.length > 1) {
                const heading = document.createElement('h2');
                heading.textContent = section.version;
                article.append(heading);
            }
            const body = document.createElement('pre');
            body.textContent = section.notes;
            article.append(body);
            container.append(article);
        }
        if (!container.children.length) container.textContent = 'No new release notes to show.';
    } catch (error) {
        const element = document.getElementById('notes-error');
        element.hidden = false;
        element.textContent = error.message || String(error);
    }
}
document.getElementById('close-notes').addEventListener('click', async () => {
    try {
        await window.desktop.api.close_whats_new();
    } catch (error) {
        const element = document.getElementById('notes-error');
        element.hidden = false;
        element.textContent = error.message || String(error);
    }
});
window.addEventListener('desktopready', loadNotes);
loadNotes();
