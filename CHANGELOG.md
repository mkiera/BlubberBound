# Changelog

Release headings use the complete version without a leading `v`. Tagged sections are kept unchanged. Stable sections collect the results of their beta releases.

## Unreleased

- Track video compression progress by encoded frames and refresh progress more frequently, reserving completion for a checked and saved output.
- Keep original resolution unless the user selects a resolution limit or manual scaling.
- Save the smallest output produced when a size limit cannot be met, with a warning showing the actual size.
- Use Compress throughout the interface and add a file-compressor description below the title.
- Open application settings from the gear button and make Advanced a distinct button.
- Explain each compression mode, disable unused controls, reset inactive quality and bitrate values, and restore the preferred encoder when leaving constant-quality mode.
- Add MKV and MOV video output, plus M4A, AAC, and Ogg audio output.

## 1.0.0-beta.3 - 2026-09-12

- Keep the GPLv3 license without requiring acceptance during installation.

## 1.0.0-beta.2 - 2026-09-12

- Use BlubberBound for the executable, installation and settings folders, shortcuts, and installer download.
- Publish only the Windows installer, without portable archives or checksum files.
- Skip app builds for documentation-only changes and avoid duplicate branch installers for release-tagged commits.

## 1.0.0-beta.1 - 2026-09-12

- Rename the application to BlubberBound and use the new icon.
- Run the desktop application with a Rust backend while retaining saved settings, queue entries, and the Windows testing launcher.
- Preview short video and audio samples or compressed images before exporting, with original and output comparison.
- Add advanced bitrate, constant-quality, scaling, frame-rate, encoder, audio, image-quality, and metadata controls.
- Squeeze completed files again, either creating another copy or replacing the previous output after compression succeeds.

## 1.0.0 - 2026-09-12

- Compress batches of videos, audio, and still images to a size limit per file, with MP4, WebM, MP3, Opus, WebP, and JPEG output.
- Add files by dropping them, selecting them, or choosing a folder, and keep the queue and settings between sessions.
- Choose hardware or software video encoding, limit resolution, and see progress and the resulting file sizes.
- Keep original files and create numbered output copies when a filename already exists, with retry, skip, and stop controls.
- Open completed files, show them in Explorer, or send video output to an installed FlipperClipper.
- Choose Stable, Beta, or Alpha in Updates, inspect release notes, and install a selected build while preserving settings.
