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
1. Update the version (e.g. `0.2.0`) in all three places, and let Cargo refresh `Cargo.lock`:
   - [`package.json`](../package.json) (and the two matching fields at the top of `package-lock.json`)
   - [`src-tauri/tauri.conf.json`](../src-tauri/tauri.conf.json)
   - [`src-tauri/Cargo.toml`](../src-tauri/Cargo.toml)
2. Commit and push the version bump:
   ```bash
   git commit -am "chore: release v0.2.0"
   git push origin main
   ```
3. Tag the commit with `v<version>` and push the tag:
   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```

The workflow creates a draft release, builds every platform into it, then publishes it (tag pushes only; a manual run leaves a draft prerelease).

### Matrix Builds
GitHub Actions builds four targets:
- **Windows x64** and **Windows ARM64** (`windows-latest`): NSIS `.exe` and `.msi`
- **macOS Apple Silicon** and **macOS Intel** (`macos-latest`, cross-compiling the Intel build): `.dmg`

`tauri-action` uploads the installers plus the signed updater artifacts and `latest.json` to the release. The in-app updater reads
`https://github.com/CubiPal/cubipal-desktop/releases/latest/download/latest.json`.

### Secrets
| Secret | Needed | Used for |
|---|---|---|
| `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Yes | Signing updater artifacts (the public key is in `tauri.conf.json`) |
| `GOOGLE_CLIENT_ID` | Optional | Overrides the built-in Google OAuth client ID |
| `GOOGLE_CLIENT_SECRET` | Optional, but Google sign-in needs it | Sent on Google token requests; for installed apps Google treats it as non-confidential |
| `MICROSOFT_CLIENT_ID` | Optional | Enables Microsoft 365 / Outlook. Without it (or with the all-zero placeholder) the app shows Outlook as "coming soon" |

These are read at compile time (`option_env!`), so a local `npm run tauri build` uses whatever is set in your shell.

### Firmware is not part of desktop releases
Pal firmware is released from the firmware repo and published on `https://cubipal.com/firmware/` (`latest.json` plus per-chip images). The desktop release no longer downloads or re-attaches firmware, and no `FW_RELEASE_PAT` is needed.

---

## 3. First-Run Installation## 3. First-Run Installation & OS Warnings (Unsigned Installers)

For the beta, the installers are distributed through [GitHub Releases](https://github.com/CubiPal/cubipal-desktop/releases/latest) without Apple Developer or Microsoft code-signing certificates. (Updates installed from inside the app are separately signed with the Tauri updater key.)

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

