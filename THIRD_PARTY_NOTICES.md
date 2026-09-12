# Third-party notices

SealSqueeze is licensed under GNU GPL version 3. See [LICENSE](LICENSE).

The application reuses code and assets from [FinFetcher](https://github.com/mkiera/FinFetcher) and follows encoding behavior from FlipperClipper. The Outfit font files come from FinFetcher.

## Fonts

Outfit is licensed under the SIL Open Font License 1.1. Its copyright and license are included in [fonts/OFL.txt](fonts/OFL.txt).

## FFmpeg

Windows packages include FFmpeg and FFprobe 8.0.1 from [Gyan's essentials build](https://github.com/GyanD/codexffmpeg/releases/tag/8.0.1). The archive SHA256 is `e2aaeaa0fdbc397d4794828086424d4aaa2102cef1fb6874f6ffd29c0b88b673`.

The original build license and readme are included in `tools/FFMPEG-LICENSE.txt` and `tools/FFMPEG-README.txt` inside the application resources. The build enables GPL components. Its readme identifies the source as [FFmpeg commit 894da5ca7d](https://github.com/FFmpeg/FFmpeg/commit/894da5ca7d). Build information and dependencies are documented by [Gyan](https://www.gyan.dev/ffmpeg/builds/).

## Rust dependencies

The application uses Tauri and Rust libraries listed in `src-tauri/Cargo.lock`. Windows packages include dependency names, versions, license declarations, and source links in `THIRD_PARTY_LICENSES.txt`. License and copyright files supplied by those libraries are copied into `licenses`.

Microsoft Edge WebView2 Runtime is installed and licensed separately by Microsoft.
