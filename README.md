# Ruza-Lang (`v0.8.0`)

[![Build & Release](https://github.com/notnedar/ruza-lang/actions/workflows/release.yml/badge.svg)](https://github.com/notnedar/ruza-lang/actions/workflows/release.yml)

> [!IMPORTANT]
> **Tested & Working In-Game (PC & Android) · Vibe-Coded Educational Project**
> - Confirmed **tested and working in-game** on both PC and Android clients (standalone APK & PC/ADB).
> - This is a **vibe-coded educational project** — future client updates or hotfixes may still cause quirks or breaking changes. Always keep automatic backups (`_ENPatchBackup\` / local APK backups) and use at your own risk.

An educational localization & asset-repacking suite demonstrating exact-size string patching (**English**, **Bahasa Indonesia**, and **Bring Your Own Custom Language Packs**) for **Windows PC** and **Android** (both a **standalone zero-PC Android APK** and **PC-to-Android ADB**), SHA-256 verification, and safe one-click rollback.

> **Pre-built official binaries:** download **`RuzaLangPatcher.exe`** (Windows PC) or **`RuzaLangPatcher.apk`** (Standalone Android — 100% no PC, no root!) from [Releases](https://github.com/notnedar/ruza-lang/releases).

---

## What's New in `v0.8.0`

- **Bring Your Own Language Pack (`BYOL` · PC, Android ADB & Standalone APK):**
  - Load any custom `.json` language pack directly inside **Files → Custom Language Pack → Load JSON** on **Windows PC**, **PC-to-Android ADB**, or **Standalone Android APK** (100% offline — no compiling required!).
  - Supports **base inheritance** (`meta.base`: `"en"`, `"id"`, or `"jp"`), flat dictionary drop-ins, and **automatic PC → Android ID remapping** so one single `.json` pack works everywhere.
  - Export a ready-to-edit starter template directly from the Windows app (**Files → Template**) or grab [`patches/custom-pack-template.json`](patches/custom-pack-template.json).
- **Community Language Contribution Kit & Validator CLI:**
  - Added [`CONTRIBUTING.md`](CONTRIBUTING.md) with a step-by-step guide for bringing your own language pack and submitting PRs for `EN`/`ID` fixes or new community languages.
  - Added zero-dependency validator [`tools/validate-pack.mjs`](tools/validate-pack.mjs) (`node tools/validate-pack.mjs <pack.json>`) checking JSON syntax, placeholder parity (`{0}`, `%s`, `|`), Unity rich-text tags, tight-button wrap warnings, and UTF-8 byte budget.
- **Combat Timing Terminology Fix (`EN` & `ID`):**
  - Replaced all `62 EN + 73 ID` literal `"Just Block / Guard / Parry / Dodge / Evade"` strings with natural action-RPG timing terms (**`Perfect Guard`**, **`Perfect Parry`**, **`Perfect Dodge`**, **`Parry Window / Celah Parry`**, **`Dodge Counter`**, **`Perfect Guard: Counter`**) and fixed `Guard` (`ID` previously `"Penjaga"`).
- **Previous Highlights (`v0.7.1` & `v0.7.0`):**
  - Repaired `6` off-by-one chunk shifts (`424` scrambled strings, including the `Equip` button), aligned `3,362` official global terms & 9 elements, applied `162` anti-wrap UI overrides, fixed Android 15–17 Edge-to-Edge insets, added automatic **Download Shizuku** popup, and made the standalone APK `100%` non-root.

---

## Preview & Usage Demo

### Standalone Android APK (`RuzaLangPatcher.apk` · Phone & Tablet)

| Portrait Phone (Home) | Portrait Phone (Files & Sync) |
| :---: | :---: |
| ![Android APK Home](docs/screenshot-apk-home.png) | ![Android APK Files](docs/screenshot-apk-files.png) |

![Android APK Landscape](docs/screenshot-apk-tablet.png)

### Desktop Live Usage Walkthrough (`EN` ↔ `ID` + Rewrite Progress)

![Ruza-Lang Usage Demo](docs/usage.webp)

### Desktop Interface Screenshots

| English (`EN`) | Bahasa Indonesia (`ID`) |
| :---: | :---: |
| ![English UI](docs/screenshot-en.png) | ![Indonesian UI](docs/screenshot-id.png) |

![Android ADB Tab](docs/screenshot-android.png)

### In-Game Samples (`EN`)

| Character Screen | Star Journey Record |
| :---: | :---: |
| ![Character Screen Sample](docs/sample-ingame-character.jpg) | ![Star Journey Record Sample](docs/sample-ingame-journey.jpg) |

![Settings Menu Sample](docs/sample-ingame-en.jpg)

---

## Features / Fitur

- **Bilingual Switch (`EN` / `ID`):** Toggle between **English** and **Bahasa Indonesia** directly inside the GUI (Windows & Android) — switches both the app interface and the target localization pack.
- **PC 4-Store Localization Pipeline (`77,661` strings):**
  - `VStringPool` — `74,055` strings (dialogue, quests, skills, items, settings)
  - `Static UI KeyConfig` — `3,135` strings (prefab labels, buttons, headers, notices)
  - `StaticPackage CSV` — `113` strings (startup & updater messages)
  - `resources.assets` — `358` strings (fallback startup & SDK UI)
- **Android 2-Bundle Pipeline (`77,190` strings · `99.4%` coverage):** `VStringPool` (`74,055`) + `Static UI KeyConfig` (`3,135`) + Android Cached News (`EN` / `ID`) — zero root and zero game APK re-signing.
- **Dedicated Translate News (`EN` / `ID`) Button:** Translates cached in-game announcement HTML pages (`16` pages in `HTTPCache` + `Library v3`) with automatic backup restore on Rollback.
- **Process Guard:** Checks whether target client/launcher processes are running (PC & Android) and blocks patching, news translation, or rollback until closed.
- **One-Click Rollback:** Saves verified original backups automatically before patching and restores exact original SHA-256 hashes anytime.
- **Exact-Size Asset Repacking:** Preserves identical file byte sizes across all targets using exact-size LZ4HC tuning and round-trip integrity checks.

---

## Usage / Cara Pakai

### Option A — Standalone Android Phone (No Laptop, No Root!)
1. Finish downloading the in-game additional data on Android, then **completely close (Force Stop) the game**.
2. Download and install **`RuzaLangPatcher.apk`** from [Releases](https://github.com/notnedar/ruza-lang/releases).
3. **Grant `Android/data` access (Android 11–17, zero PC, zero root):**
   - **Recommended — Shizuku via Wireless Debugging:**
     1. Install official **[Shizuku](https://shizuku.rikka.app/download/)** (Google Play or GitHub).
     2. Enable **Developer Options → Wireless Debugging** on your phone.
     3. Open Shizuku → tap **Pairing** → enter the 6-digit code in the notification → tap **Start**.
     4. Open **Ruza-Lang** → tap **Setup Access** → tap **Authorize Shizuku** → choose **Allow all the time**.
   - **Alternative — Folder Picker (SAF) / Direct Storage:** Works directly on Android 8–12 or compatible emulators without Shizuku (100% non-root).
4. Choose **`EN`** (English) or **`ID`** (Bahasa Indonesia) in the top bar (or left rail on tablets/landscape).
5. Tap **Install Patch (`EN` / `ID`)** — takes ~3 seconds with automatic local backups and full SHA-256 verification.
6. Tap **Rollback JP** anytime to restore original files (including cached News if backed up).

### Option B — Windows PC Client
1. Make sure the target client and launcher are completely closed.
2. Download and run **`RuzaLangPatcher.exe`** from [Releases](https://github.com/notnedar/ruza-lang/releases).
3. Check the auto-detected target folder in the bottom dock (or click the path / open **Files** to browse manually).
4. Choose **`EN`** or **`ID`** at the bottom of the left rail and click **Install Patch**.
5. *(Optional)* After opening the in-game News/Announcements screen once, close the client and click **Translate News (`EN` / `ID`)**.
6. Click **Rollback JP** anytime to restore original files.

### Option C — Patch Android from Windows PC (ADB: USB / Wi-Fi / Emulator)
Works on any real Android phone (USB or Android 11+ Wireless Debugging) or Windows Android emulator (BlueStacks, LDPlayer, MuMu, MEmu, Nox):
1. Finish downloading in-game data and **completely close the game** on Android.
2. Enable **USB Debugging** (or **Wireless Debugging** / Emulator ADB setting) and connect to your PC.
3. Open `RuzaLangPatcher.exe`, switch the bottom dock toggle to **Android** (or click **ADB Guide `?`** for step-by-step setup & Wireless `IP:Port` connect), choose **`EN`** or **`ID`**, and click **Install Patch**.

### Option D — Bring Your Own Language Pack (BYOL) & Language Contributions
Want to play in another language (Spanish, Portuguese, French, Vietnamese, Thai, Russian, etc.), tweak wording yourself, or help improve `EN` / `ID`?
- **Use Your AI Agent / Gemini / ChatGPT:** We live in the age of LLMs — just point your coding agent, **Gemini**, or **ChatGPT** at this repository (`https://github.com/notnedar/ruza-lang`) and ask it to generate a translation pack from `patches/custom-pack-template.json` / `patches/full-en-3219598.json` + `patches/static-ui-en-3219598.json` and validate it with `node tools/validate-pack.mjs`! Copy-paste prompts are ready in **[`CONTRIBUTING.md`](CONTRIBUTING.md)**.
- **Load in 1 Click (PC & Android):** Open **Files → Custom Language Pack → Load JSON** on **Windows PC** or **Android APK**, then click **Install Patch (`<CODE>`)**! One `.json` pack works across both PC and Android automatically (set `"base": "en"`, `"id"`, or `"jp"` for untranslated keys).
- **Contribute Back:** Open a Pull Request with your `EN`/`ID` fixes or community language pack — see **[`CONTRIBUTING.md`](CONTRIBUTING.md)** for full details.

---

## Repository Layout

- `src/` — Standalone Windows desktop launcher & frameless WebView2 UI (`src/ui/index.html`, `src/ui/hero.webp`)
- `android/` — Standalone universal Android APK project (Kotlin + Shizuku AIDL + responsive WebView UI)
- `resources/` — Shared compressed bilingual localization payloads (`EN` + `ID`, used by both PC and Android builds)
- `patches/` — Human-readable JSON translation dictionaries (`EN` + `ID`)
- `tools/` — Node.js scripts for payload packing and asset repacking research

---

## Build Transparency & Verification

Official release binaries (`RuzaLangPatcher.exe` and `RuzaLangPatcher.apk`) are compiled directly on **GitHub Actions** (`windows-latest` & `ubuntu-latest`) from this public source tree via [`.github/workflows/release.yml`](.github/workflows/release.yml) and signed with **GitHub Artifact Build Provenance Attestation**.

Verify both binaries locally:

```bash
gh attestation verify RuzaLangPatcher.exe --owner notnedar
gh attestation verify RuzaLangPatcher.apk --owner notnedar
```

---

## Build from Source

### Windows PC Executable
Requires Windows 10/11 (x64), MSVC build tools, and Edge WebView2 Runtime:

```powershell
cargo build --release
```

Output: `target\release\RuzaLangPatcher.exe`

### Standalone Android APK
Requires JDK 17 and Android SDK (Platform 36):

```bash
cd android
./gradlew testDebugUnitTest assembleRelease
```

Output: `android/app/build/outputs/apk/release/app-release.apk`

---

## Disclaimer

This repository is strictly an **educational project** created to study binary serialization, compression, and localization tooling. No copyrighted or proprietary assets are distributed here.
