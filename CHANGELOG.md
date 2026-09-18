# Changelog

Release headings use the complete version without a leading `v`. Tagged sections are kept unchanged. Stable sections collect the results of their beta releases.

## Unreleased

## 1.1.1 - 2026-09-18

- Keep output formats selectable in Auto quality and use the selected format for saved files.

## 1.1.0 - 2026-09-15

- Add Auto quality compression for each queued file. It searches for smaller video output that passes frame comparisons, checks image pixels and audio samples exactly, and keeps the original when no smaller output passes.
- Choose Auto video quality from short sections across the file, then verify the encoded output. Limit long videos to one full encode plus one retry, remove a redundant full decode, and show the current pass and percentage.
- Bundle FFmpeg 9.0 so copied AAC audio stays aligned in Auto quality's MKV output.
- Keep the Advanced button in the heading row when Auto quality is selected, and leave it disabled. Remove the compression mode description and add space before Size limit.
- Fix in-app alpha installation by accepting GitHub's signed artifact storage redirect from nightly.link without sending credentials to the storage host.
- Show each completed compression's elapsed time in minutes and seconds, and retain it after restarting the app.

## 1.1.0-beta.3 - 2026-09-15

- Speed up Auto video verification by counting output frames during the comparison, removing a second full decode.
- Choose Auto settings with a quality margin and limit long videos to one full encode plus one retry. Show the current pass and percentage during encoding and verification.

## 1.1.0-beta.2 - 2026-09-15

- Fix Auto quality sometimes keeping the original despite a smaller valid output. Search short sections across longer videos to choose quality faster.
- Bundle FFmpeg 9.0 so copied AAC audio stays aligned in Auto quality's MKV output.
- Keep the Advanced button in the heading row when Auto quality is selected, and leave it disabled. Remove the compression mode description and add space before Size limit.

## 1.1.0-beta.1 - 2026-09-15

- Add Auto quality compression for each queued file. It searches for the smallest video candidate that passes frame comparisons, checks image pixels and audio samples exactly, and keeps the original when no smaller output passes.
- Fix in-app alpha installation by accepting GitHub's signed artifact storage redirect from nightly.link without sending credentials to the storage host.

## 1.0.0-beta.5 - 2026-09-12

- Fix in-app updates by exiting promptly after starting the installer and allowing older versions to close before replacing application files.
- Test running-application upgrades and relaunch during Windows packaging.
## 1.0.0-beta.4 - 2026-09-12

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

## 1.0.0 - 2026-09-13

- Compress batches of videos, audio, and still images to a size limit per file, with MP4, WebM, MP3, Opus, WebP, and JPEG output.
- Add files by dropping them, selecting them, or choosing a folder, and keep the queue and settings between sessions.
- Choose hardware or software video encoding, limit resolution, and see progress and the resulting file sizes.
- Keep original files and create numbered output copies when a filename already exists, with retry, skip, and stop controls.
- Open completed files, show them in Explorer, or send video output to an installed FlipperClipper.
- Choose Stable, Beta, or Alpha in Updates, inspect release notes, and install a selected build while preserving settings.
- Rename the application to BlubberBound and use the new icon.
- Run the desktop application with a Rust backend while retaining saved settings, queue entries, and the Windows testing launcher.
- Preview short video and audio samples or compressed images before exporting, with original and output comparison.
- Add advanced bitrate, constant-quality, scaling, frame-rate, encoder, audio, image-quality, and metadata controls.
- Compress completed files again, either creating another copy or replacing the previous output after compression succeeds.
- Use BlubberBound for the executable, installation and settings folders, shortcuts, and installer download.
- Publish only the Windows installer, without portable archives or checksum files.
- Skip app builds for documentation-only changes and avoid duplicate branch installers for release-tagged commits.
- Keep the GPLv3 license without requiring acceptance during installation.
- Track video compression progress by encoded frames and refresh progress more frequently, reserving completion for a checked and saved output.
- Keep original resolution unless the user selects a resolution limit or manual scaling.
- Save the smallest output produced when a size limit cannot be met, with a warning showing the actual size.
- Use Compress throughout the interface and add a file-compressor description below the title.
- Open application settings from the gear button and make Advanced a distinct button.
- Explain each compression mode, disable unused controls, reset inactive quality and bitrate values, and restore the preferred encoder when leaving constant-quality mode.
- Add MKV and MOV video output, plus M4A, AAC, and Ogg audio output.
- Fix in-app updates by exiting promptly after starting the installer and allowing older versions to close before replacing application files.
- Test running-application upgrades and relaunch during Windows packaging.
