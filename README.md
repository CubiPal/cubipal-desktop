# CubiPal Desktop Companion

A lightweight tray application for Windows and macOS that connects your digital workday to your [CubiPal](https://cubipal.com) modular desk pet over your local Wi-Fi.

![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)

---

## Features

- **Local-First & Private:** All communication between this app and your CubiPal happens directly over your local Wi-Fi network via HTTP. No calendar data ever touches a CubiPal cloud server; no accounts are required.
- **Autonomous Device Reminders:** The app pushes your next two meetings to your CubiPal. The device runs its own countdown timer and sounds reminder chimes even if your laptop sleeps.
- **Calendar Synchronization:** Read-only OAuth 2.0 PKCE synchronization with **Google Calendar**, plus private **iCal (ICS)** calendar links (time zones, all-day and recurring events). **Microsoft 365 / Outlook** support is coming soon (the app shows it as "Coming Soon"; see [docs/building-and-releasing.md](docs/building-and-releasing.md) for the `MICROSOFT_CLIENT_ID` build setting that will enable it).
- **Private Mode:** Optionally replace event titles with "Busy" before sending them to the device.
- **Focus / Countdown Timers:** Trigger preset (30m, 60m) or custom (1-180 min) focus sessions right from the system tray.
- **Snooze Alerts:** The tray can hold back meeting alerts for an hour, with "Resume Alerts" to end it early.
- **Zero-Friction Physical Pairing:** Device pairing requires physical proximity by tapping a simple touch pattern on your CubiPal's touch point (the flat top of its head) to reveal a one-time setup code.

---

## Architecture & Tech Stack

- **Framework:** [Tauri v2](https://v2.tauri.app/) (Rust core + Web frontend)
- **Security:** Platform-native credential management via **Windows Credential Manager** and **macOS Keychain** (`keyring` crate)
- **Local Discovery:** mDNS (`_cubipal._tcp`) service discovery with manual IP fallback
- **Cross-Platform:** Windows 10/11 (x64, ARM64) and macOS 11 or later (Apple Silicon, Intel)

The full technical design is in the CubiPal tech design document (internal).

---

## Releases & Auto-Updates

**Download:** [github.com/CubiPal/cubipal-desktop/releases/latest](https://github.com/CubiPal/cubipal-desktop/releases/latest)

- **Multi-Platform Releases:** Every `v*` tag builds installers for Windows (x64, ARM64) and macOS (Apple Silicon, Intel).
- **Auto-Updater:** With "Automatic Updates" on (General tab), the app checks this repo's latest release for a signed update manifest (`latest.json`) 30 s after startup and daily, and asks before installing. The tray's "Check for Updates..." works either way.
- **Pal Firmware Updates:** The app reads `https://cubipal.com/firmware/latest.json`, picks the build for your Pal's board (`esp32s3` or `esp32`), and only installs it after checking its size and SHA-256 and the image header. It won't start while a focus timer is running on your Pal or a meeting is about to start, or if it can't confirm the Pal is idle. Firmware is not attached to desktop releases.

See [docs/building-and-releasing.md](docs/building-and-releasing.md) to cut a release.

*Code signing: the installers are not yet signed by Apple or Microsoft, so both systems warn on first launch. On Windows choose **More info -> Run anyway**; on macOS open **System Settings -> Privacy & Security** and click **Open Anyway**. (In-app updates are signed with the Tauri updater key and verified before they install.)*

---

## Development Setup

### Prerequisites

1. **Node.js** (v18+ recommended)
2. **Rust** (`rustup`, stable toolchain):
   - Windows: [rustup.rs](https://rustup.rs/) and Visual Studio C++ Build Tools
   - macOS: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
3. **CubiPal Hardware** on your local Wi-Fi network. Firmware 1.1.0 or newer reports its board and device id, which the app needs for over-the-air updates and to confirm it's talking to your Pal.

### Getting Started

```bash
# Clone the repository
git clone https://github.com/CubiPal/cubipal-desktop.git
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
