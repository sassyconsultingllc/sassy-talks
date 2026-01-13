# Sassy-Talk Desktop (Windows/macOS/Linux)

Cross-platform PTT walkie-talkie with retro vibes. Built with Tauri 2.0.

## Requirements

- Node.js 18+
- Rust 1.70+
- Platform-specific:
  - **Windows**: Visual Studio Build Tools 2019+
  - **macOS**: Xcode Command Line Tools
  - **Linux**: `build-essential`, `libwebkit2gtk-4.1-dev`, `libasound2-dev`

## Quick Start

```bash
# Install dependencies
npm install

# Development mode
cargo tauri dev

# Production build
cargo tauri build
```

## Build Outputs

| Platform | Location |
|----------|----------|
| Windows | `src-tauri/target/release/bundle/msi/` |
| macOS | `src-tauri/target/release/bundle/dmg/` |
| Linux | `src-tauri/target/release/bundle/appimage/` |

## Features

- Push-to-talk voice communication
- 16 virtual channels
- End-to-end encryption (AES-256-GCM)
- Opus codec for low-latency audio
- UDP multicast discovery
- Retro walkie-talkie UI

## Project Structure

```
tauri-desktop/
├── src/                 # React frontend
│   ├── App.tsx          # Main UI component
│   ├── main.tsx         # React entry
│   └── styles/          # CSS
├── src-tauri/           # Rust backend
│   ├── src/
│   │   ├── main.rs      # Tauri entry
│   │   ├── lib.rs       # Core library
│   │   ├── commands.rs  # IPC handlers
│   │   ├── audio.rs     # Audio engine
│   │   ├── codec.rs     # Opus codec
│   │   ├── protocol.rs  # Wire protocol
│   │   ├── security/    # Crypto & security
│   │   └── transport/   # Networking
│   ├── Cargo.toml
│   └── tauri.conf.json
├── package.json
└── index.html
```

## License

© 2025 Sassy Consulting LLC. All rights reserved.
