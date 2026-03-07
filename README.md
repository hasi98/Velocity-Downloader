# 🚀 Velocity Downloader

A high-performance, modern download manager built with **Tauri**, **React**, and **Rust**. Velocity Downloader provides a seamless downloading experience with multi-threaded support and deep browser integration via a dedicated Chrome extension.

![My IDM Banner](https://raw.githubusercontent.com/your-username/velocity-downloader/main/public/banner.png) *(Note: Add a real banner later)*

## ✨ Features

- **Blazing Fast Downloads**: Multi-threaded downloading for maximum speed.
- **Browser Integration**: Automatic download interception via the Chrome Extension.
- **Native Performance**: Built with Rust for efficiency and low memory footprint.
- **Modern UI**: Sleek, dark-themed interface built with React and Tailwind CSS.
- **Queue Management**: Pause, resume, and prioritize your downloads.
- **Easy Installation**: Single executable for Windows, macOS, and Linux.

## 🛠️ Tech Stack

- **Frontend**: React, TypeScript, Vite, CSS (Vanilla/Modules)
- **Backend/Core**: Tauri (Rust)
- **Extension**: Manifest V3 (Javascript)

## 🚀 Getting Started

### Prerequisites

- [Node.js](https://nodejs.org/) (Latest LTS)
- [Rust](https://www.rust-lang.org/tools/install)
- [WebView2](https://developer.microsoft.com/en-us/microsoft-edge/webview2/) (For Windows users)

### Installation for Developers

1. **Clone the repository:**
   ```bash
   git clone https://github.com/your-username/velocity-downloader.git
   cd velocity-downloader
   ```

2. **Install dependencies:**
   ```bash
   npm install
   ```

3. **Run the app in development mode:**
   ```bash
   npm run tauri dev
   ```

### Installing the Browser Extension

1. Open Chrome and go to `chrome://extensions/`.
2. Enable **Developer mode** (top right).
3. Click **Load unpacked**.
4. Select the `extension` folder from this project directory.

## 📦 Building for Production

To create a standalone installer (EXE for Windows):

```bash
npm run tauri build
```

The installer will be generated in `src-tauri/target/release/bundle/`.

## 🤝 Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

---

Built with ❤️ by [Your Name]
