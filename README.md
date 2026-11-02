# CubiPal Desktop Companion

A lightweight tray application for Windows and macOS that connects your digital workday to your [CubiPal](https://cubipal.com) modular desk pet over your local Wi-Fi.

![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)

---

## Features

- **Local-First & Private:** All communication between this app and your CubiPal happens directly over your local Wi-Fi network via HTTP. No calendar data ever touches a CubiPal cloud server; no accounts are required.
- **Autonomous Device Reminders:** The app pushes your next two meetings to your CubiPal. The device runs its own countdown timer and sounds reminder chimes even if your laptop sleeps.
- **Calendar Synchronization:** Read-only OAuth 2.0 PKCE synchronization with **Google Calendar** and **Microsoft 365 / Outlook**, plus support for private **iCal (ICS)** calendar links.
- **Private Mode:** Optionally replace event titles with "Busy" before sending them to the device.
- **Focus / Countdown Timers:** Trigger preset (30m, 60m) or custom focus sessions right from the system tray.
- **Zero-Friction Physical Pairing:** Device pairing requires physical proximity by tapping a simple touch pattern on your CubiPal's crown to reveal a one-time setup code.

---

## Architecture & Tech Stack

- **Framework:** [Tauri v2](https://v2.tauri.app/) (Rust core + Web frontend)
- **Security:** Platform-native credential management via **Windows Credential Manager** and **macOS Keychain** (`keyring` crate)
- **Local Discovery:** mDNS (`_cubipal._tcp`) service discovery with manual IP fallback
- **Cross-Platform:** Windows 10/11 (x64, ARM64) and macOS 13+ (Apple Silicon, Intel)

Full technical design specifications are available in the [CubiPal Tech Design Document](https://github.com/houndman1017/CubiPal/blob/main/docs/desktop-app-tech-design.md).

---

## Releases & Auto-Updates

The companion app features automated multi-platform release CI and in-app auto-updating powered by Tauri v2:

- **Multi-Platform Releases:** Every `v*` tag trigger builds signed installers for Windows (x64, ARM64) and macOS (Apple Silicon, Intel).
- **Auto-Updater:** The desktop app checks GitHub Releases for verified signed update manifests (`latest.json`) on startup and daily, prompting before installing updates.
- **Safety Pause:** Updates pause automatically if a focus timer or meeting countdown is active on your desk pet to avoid interruptions.
- **Firmware Asset Linking:** GitHub Actions release workflows automatically attach compatible firmware binaries (`.bin` and `.sha256`) from `CubiPal` releases for OTA flashing.

### Creating a New Release

```bash
# 1. Update version across package.json, src-tauri/Cargo.toml, and src-tauri/tauri.conf.json
# 2. Commit and tag:
git tag v0.2.0
git push origin v0.2.0
```

*Note on code signing: Early v1 releases are unsigned. Follow the included instructions on macOS (Control-Click -> Open) or Windows (More Info -> Run anyway) on first launch.*

---

## Development Setup

### Prerequisites

1. **Node.js** (v18+ recommended)
2. **Rust** (`rustup`, stable toolchain):
   - Windows: [rustup.rs](https://rustup.rs/) and Visual Studio C++ Build Tools
   - macOS: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
3. **CubiPal Hardware:** Running firmware v1.0+ on your local Wi-Fi network

### Getting Started

```bash
# Clone the repository
git clone https://github.com/houndman1017/cubipal-desktop.git
cd cubipal-desktop

# Install frontend dependencies
npm install

# Run in development mode (spawns tray app with hot-reload)
npm run tauri dev

# Build production bundle
npm run tauri build
```

---

## Privacy Policy

The companion app adheres strictly to local-first privacy principles. Read our full policy at [https://cubipal.com/privacy](https://cubipal.com/privacy).

---

## License

This project is licensed under the [MIT License](LICENSE).
