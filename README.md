# Velocity Downloader

Velocity Downloader is a Windows-focused download manager built with Tauri, Rust, React, and TypeScript. It is designed for fast segmented downloads, browser capture, persistent download history, and an IDM-style native desktop workflow.

Download Windows Installer: https://github.com/hasi98/Velocity-Downloader/releases/latest

## Features

- Multi-segment downloads with pause, resume, stop, and retry-ready task state.
- IDM-style Add Download window with automatic URL analysis.
- Hidden pre-download/prefetch after analysis, with final save only after user confirmation.
- Compact download progress window by default, with More/Less segment details.
- Per-download speed limit controls and global speed limit support for new downloads.
- Persistent download history after app restart.
- Resume support after restart using `.meta` files and verified temp segments.
- Automatic duplicate filename handling such as `file (1).zip`.
- File rename before starting a download.
- Download categories by file type.
- Batch download window with pasted URLs, imported URL lists, extension filtering, and queue modes.
- Browser extension integration for Chrome, Edge, Brave, Opera, and Vivaldi-style Chromium browsers.
- Browser download interception with cookies, referer, and user-agent forwarding.
- "Download with Velocity" right-click browser menu.
- Browser fallback if Velocity cannot accept an intercepted download.
- Tray icon support with hide-on-close behavior.
- Optional "Start with Windows" setting.
- Signed auto-update support through Tauri updater and GitHub Releases.
- Native child-window placement so Add Download, Settings, Batch, Extension, and progress windows open over the main app.
- Updated app, taskbar, titlebar, tray, installer, and web favicon icons.

## Tech Stack

- Frontend: React, TypeScript, Vite, CSS
- Desktop shell: Tauri 2
- Core downloader: Rust
- Browser extension: Manifest V3 JavaScript
- Local app bridge: HTTP server on `127.0.0.1:41420`

## Requirements

- Windows 10/11
- WebView2 Runtime
- Node.js LTS
- Rust stable toolchain
- Visual Studio Build Tools with the Windows MSVC toolchain

## Development

Clone the repository:

```bash
git clone https://github.com/hasi98/Velocity-Downloader.git
cd Velocity-Downloader/velocity-downloader
```

Install dependencies:

```bash
npm install
```

Run the app in development mode:

```bash
npm run tauri dev
```

Build the frontend only:

```bash
npm run build
```

Check the Rust backend:

```bash
cd src-tauri
cargo check
```

## Production Build

Create the Windows executable and installers:

```bash
npm run tauri build
```

Updater release builds must be signed with the private updater key:

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -Raw "$HOME\.tauri\velocity-downloader.key"
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "<your-updater-key-password>"
npm run tauri build
npm run updater:manifest -- --tag=vX.Y.Z
```

Build outputs are generated here:

- Standalone executable: `src-tauri/target/release/velocity-downloader.exe`
- NSIS installer: `src-tauri/target/release/bundle/nsis/`
- MSI installer: `src-tauri/target/release/bundle/msi/`
- Updater manifest: `src-tauri/target/release/bundle/latest.json`

For GitHub Releases, upload the NSIS installer, its `.sig` file, and `latest.json`. The app checks:

```text
https://github.com/hasi98/Velocity-Downloader/releases/latest/download/latest.json
```

## Browser Extension

The extension files are in the `extension` directory.

Manual installation:

1. Open your Chromium browser extension page, for example `chrome://extensions`, `edge://extensions`, or `brave://extensions`.
2. Enable Developer mode.
3. Click Load unpacked.
4. Select the `extension` folder from this project.
5. Keep Velocity Downloader running so the extension can reach `http://127.0.0.1:41420`.

The extension can intercept normal browser downloads and can also send links through the "Download with Velocity" context menu.

## App Behavior

- Closing the main window hides the app to the tray instead of quitting.
- Use the tray menu to show Velocity Downloader again or quit completely.
- When Start with Windows is enabled in Settings, the app starts minimized in the background.
- Incomplete downloads are restored from `.meta` files on startup when possible.
- Completed downloads remain visible in history after restarting the app.

## Project Structure

```text
velocity-downloader/
  extension/              Browser extension
  logo/                   Source logo assets
  public/                 Frontend public assets
  src/                    React/TypeScript frontend
  src-tauri/              Rust/Tauri backend
  src-tauri/icons/        Generated app icons
```

## Notes

- Some protected streaming or blob-based media URLs may not be directly downloadable yet.
- Some sites require valid cookies, referer, and user-agent headers; the extension forwards these when possible.
- If Windows shows an old taskbar icon after updating, unpin the old app and pin the rebuilt executable again because Windows caches pinned icons.

## License

This project is licensed under the MIT License. See `LICENSE` for details.
