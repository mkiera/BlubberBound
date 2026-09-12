# BlubberBound

<img src="icon.png" alt="BlubberBound icon" width="128">

Compress local videos, audio, and still images to a size limit per file. BlubberBound uses Rust, Tauri, and FFmpeg with an HTML, CSS, and JavaScript interface.

## Download and install

BlubberBound is in development. The repository is private and has no published stable release yet. Current application source is on [`beta`](https://github.com/mkiera/BlubberBound/tree/beta). `main` contains the project documentation until the first stable release.

1. Open [Build Test](https://github.com/mkiera/BlubberBound/actions/workflows/build-test.yml?query=branch%3Abeta) and choose a successful run from `beta`.
2. Download the `SealSqueeze-Setup` artifact at the bottom of the run page and extract it. GitHub sign-in and repository access are required. Artifacts expire after 30 days.
3. Run `SealSqueeze-Setup.exe`. It installs for your Windows account without administrator access.
4. Open BlubberBound, add files, choose a size limit, and press **Start squeezing**.

Tagged builds will appear on the [Releases page](https://github.com/mkiera/BlubberBound/releases). Installer and data-folder names retain `SealSqueeze` for compatibility.

Microsoft Edge WebView2 Runtime is required. FFmpeg and FFprobe are included in Windows packages. The portable ZIP contains the same application. Extract the whole folder before running `SealSqueeze.exe`.

The repository is private during development. Releases require repository access. In-app updates use an existing GitHub CLI login or a `SEALSQUEEZE_GITHUB_TOKEN` environment variable with access to the repository. Credentials are not bundled into the application.

If the repository becomes public, update discovery uses anonymous GitHub requests and alpha downloads use nightly.link. No application setting needs to change.

## Compression

| Media | Output | Controls |
| --- | --- | --- |
| Video | MP4 or WebM | Size limit, bitrate or constant quality, scaling, maximum frame rate, encoder speed, audio controls |
| Audio | MP3 or Opus | Size limit or bitrate, channels, sample rate |
| Still images | WebP or JPEG | Size limit, scaling, quality, lossless WebP |

Size limits use decimal MB: 1 MB is 1,000,000 bytes. In basic mode or Advanced **Fit size limit** mode, a successful output fits the chosen limit. If the media cannot fit, its queue row reports an error. Small limits can reduce quality or resolution. Advanced manual bitrate and constant-quality modes use your chosen settings without enforcing a file-size limit.

Add several files or choose a folder. Folder selection includes files directly in that folder. Subfolders are not scanned. Each job is processed in order. Stop the queue, skip a file, or retry failed jobs. Queue entries and settings are saved between sessions.

Copies go beside their source files unless you choose an output folder. Original files are preserved. Existing output filenames receive a number. Completed files can be opened, shown in Explorer, or sent to FlipperClipper when it is installed.

Choose **Squeeze again** on a completed file to apply changed settings. **Another copy** uses a new filename in the current output folder. **Replace previous squeeze** replaces the prior output only after the new export succeeds. Changing output format requires another copy. **Cancel** leaves the file unchanged.

Hardware video encoding probes available NVIDIA, Intel, and AMD encoders and falls back to software when needed. WebM uses software encoding. Actual speed depends on the input, encoder, and requested size.

Animated images are rejected so frames are not silently lost. Video exports keep the primary video and audio streams unless audio removal is enabled. Subtitles and chapters are omitted. Metadata is removed by default and can be retained where supported through Advanced settings. JPEG output flattens transparency. Basic mode preserves compatible videos that already meet the size and resolution limits without re-encoding them.

## Quality preview

1. Add a file and choose its compression settings.
2. Select **Preview quality** on its queue row.
3. Choose a start time and a sample length of 1 to 15 seconds, then select **Generate preview**.
4. Compare the original and compressed sample, or open the sample in your media player.

Video and audio previews encode only the selected interval. Their bitrate and resolution are calculated from the full source duration and target size. Image previews process the image with the same settings as an export. The queue stays unchanged, and preview files are temporary.

A sample shows the selected scene. Other scenes can need more data, and final size-fitting retries can lower quality. Regenerate a preview after changing settings. For compatible videos already within the basic limit, the preview reports that the full export will retain original quality.

## Advanced settings

Open **Advanced** and enable **Use advanced settings**. Turn it off to return to basic compression while retaining your advanced values.

- **Fit size limit** adjusts bitrate and quality to meet the limit. **Manual bitrate** uses the selected video and audio rates. **Constant quality** uses software video encoding with CRF, where lower values retain more detail. Audio uses its bitrate setting in either manual mode.
- Scale video or images from 10% to 200%, set maximum dimensions, or reduce video frame rate. Dimensions preserve aspect ratio. Explicit scaling or dimensions prevent automatic resolution reduction to meet a small size limit.
- Choose encoding speed, audio channels, sample rate, or video audio removal. Codec restrictions can reject incompatible bitrate and sample-rate combinations. Opus uses 48,000 Hz from the available sample rates.
- Set image quality, enable lossless WebP, or retain supported metadata. In size-limit mode, image quality is an upper bound. Lossless WebP preserves the pixels after any requested resizing.

## Updates and stored data

Stable lists finished releases. Beta includes tagged betas and stable releases. Alpha lists the newest successful build of each live branch and requires manual installation. Alpha artifacts expire after 30 days.

Updates preserve the installation folder and shortcut selection. The installer keeps the previous application payload until replacement succeeds. Queue state and settings live in `%LOCALAPPDATA%\SealSqueeze`, separately from the installed application. Uninstalling retains those settings and your media files.

`app_profile.json` holds the display name and repository and package identities. To change the temporary product name, edit `display_name` and rebuild. The storage ID, package ID, executable name, installer asset, workflow filename, and artifact name stay unchanged so existing installations and updates keep working. Changing those identifiers requires a migration.

Replace `icon.png` to change the application icon. The build generates the Windows icon from that image and uses it for the executable and installer. The same PNG appears in the application header.

## Run from source

Install Node.js 22 or newer, Rust through rustup, and Visual Studio C++ Build Tools with the Windows SDK. Double-click `run.bat`, or run:

```bat
git clone --branch beta https://github.com/mkiera/BlubberBound.git
cd BlubberBound
run.bat
```

The launcher installs JavaScript build dependencies, downloads the pinned FFmpeg archive, verifies its SHA256, compiles the Rust application, and opens it. The first build downloads Rust dependencies and about 106 MB of media tools. Later runs reuse compiled dependencies. Python is not required.

For development with automatic recompilation:

```powershell
npm ci
powershell -NoProfile -ExecutionPolicy Bypass -File prepare_tools.ps1
node scripts/build-identity.mjs
npm run dev -- --config src-tauri/build-config.json
```

Pass file paths as arguments to add them to the queue:

```powershell
run.bat "C:\Videos\recording.mp4" "C:\Pictures\photo.png"
```

FFmpeg and FFprobe can also be supplied on PATH or through `SEALSQUEEZE_FFMPEG` and `SEALSQUEEZE_FFPROBE`. `--state-dir` selects a separate application data folder for testing. Development builds include webview developer tools.

## Tests and packages

Run the test suites after installing the build dependencies and media tools:

```powershell
npm run build:frontend
npm test
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

Install Inno Setup 6, then run `build.bat`. It runs the tests, compiles an optimized Rust executable, and produces `dist_installer\SealSqueeze-Setup.exe` and a portable ZIP. The application interface is embedded in the executable. Media tools are placed beside it in `tools`.

`version.txt` stores the upcoming three-part core version. Generated `build_info.json` records the actual build version, commit, branch, and workflow run ID. Release tags override the build identity without editing `version.txt`.

Feature work enters `beta`. Stable releases merge `beta` into `main` with `--no-ff`. Lightweight stable tags use `vMAJOR.MINOR.PATCH` and beta tags use `vMAJOR.MINOR.PATCH-beta.N`. `CHANGELOG.md` supplies release and in-app notes.

Every pushed branch runs `Build Test` and uploads the `SealSqueeze-Setup` artifact. `Build and Release` validates release tags and their branch placement before packaging. Its manual dispatch builds the requested version without publishing a release.

## License

GNU GPL version 3. See [LICENSE](LICENSE) and [third-party notices](THIRD_PARTY_NOTICES.md).

## Issues and related apps

Report bugs or request features in [Issues](https://github.com/mkiera/BlubberBound/issues). Include the application version, steps to reproduce, selected compression settings, and the exact error. Remove personal file paths and private media before sharing logs or samples.

[FinFetcher](https://github.com/mkiera/FinFetcher) downloads media. [FlipperClipper](https://github.com/mkiera/FlipperClipper) trims video. BlubberBound compresses local files and can send completed videos to an installed FlipperClipper.
