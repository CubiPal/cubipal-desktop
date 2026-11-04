# Building and Releasing CubiPal Desktop Companion

This guide documents the build process, release pipeline, and installation instructions for the **CubiPal Desktop Companion App** ([CubiPal/cubipal-desktop](https://github.com/CubiPal/cubipal-desktop)).

---

## 1. Local Development & Building

### Prerequisites
- **Node.js**: v20 or newer
- **Rust**: stable toolchain (`rustup default stable`)
- **Tauri CLI**: installed automatically via `npm install` (`@tauri-apps/cli`)
- **Platform Dependencies**:
  - **Windows**: Microsoft Visual Studio C++ Build Tools (or Build Tools for Visual Studio 2022) with "Desktop development with C++"
  - **macOS**: Xcode Command Line Tools (`xcode-select --install`)

### Commands
```bash
# Install frontend dependencies
npm install

# Run in development mode (hot-reload frontend + live tray)
npm run tauri dev

# Build production bundle for current OS
npm run tauri build
```

Production installers are placed in `src-tauri/target/release/bundle/`:
- Windows: `.msi` and `.exe` (NSIS installer) in `bundle/msi/` and `bundle/nsis/`
- macOS: `.dmg` and `.app` in `bundle/dmg/` and `bundle/macos/`

---

## 2. GitHub Actions Release Pipeline

The automated release workflow is defined in [`.github/workflows/release.yml`](../.github/workflows/release.yml).

### Triggering a Release
To create and publish a new release:
1. Update `"version"` in both [`package.json`](../package.json) and [`src-tauri/tauri.conf.json`](../src-tauri/tauri.conf.json) (e.g. `0.1.0`).
2. Commit and push the version bump:
   ```bash
   git commit -am "chore: release v0.1.0"
   git push origin main
   ```
3. Tag the commit with `v<version>` and push the tag:
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

### Matrix Builds
GitHub Actions automatically runs a 3-platform matrix build:
- **Windows (x64)**: `windows-latest` producing NSIS `.exe` and `.msi`
- **macOS (Apple Silicon - aarch64)**: `macos-latest` producing `.dmg`
- **macOS (Intel - x86_64)**: `macos-13` producing `.dmg`

Once all jobs complete, `tauri-action` publishes the binaries and SHA256 checksums to [GitHub Releases](https://github.com/CubiPal/cubipal-desktop/releases).

---

## 3. First-Run Installation & OS Warnings (v1 Unsigned Binaries)

As established for v1 MVP/beta, the app binaries are distributed directly via GitHub Releases without paid Apple Developer or Microsoft Store code-signing certificates.

### Windows (Microsoft Defender SmartScreen)
1. Download `CubiPal_<version>_x64-setup.exe` or `.msi`.
2. When launching the installer, Windows SmartScreen may show:
   > *"Windows protected your PC: Microsoft Defender SmartScreen prevented an unrecognized app from starting."*
3. Click **"More info"**.
4. Click **"Run anyway"**.
5. The installer completes and starts CubiPal in the system tray.

### macOS (Apple Gatekeeper)
1. Download `CubiPal_<version>_<arch>.dmg`.
2. Open the `.dmg` and drag `CubiPal.app` to your `/Applications` folder.
3. On the first launch, macOS will say:
   > *"CubiPal cannot be opened because the developer cannot be verified."*
4. Open **System Settings** > **Privacy & Security**.
5. Scroll down to the **Security** section and click **"Open Anyway"** next to CubiPal.
6. Click **"Open"** when prompted. Subsequent launches will open directly from your menu bar without warnings.

