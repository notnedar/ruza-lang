# Contributing & Bring Your Own Language Pack (BYOL)

Welcome to **Ruza-Lang**! There are **two ways** to use custom translations or contribute your language:

1. **[Bring Your Own Language Pack (No Coding Required)](#1-bring-your-own-language-pack-byol)** — create or edit a single `.json` file and load it directly in **`RuzaLangPatcher.exe`** (Windows PC + Android ADB) or **`RuzaLangPatcher.apk`** (Standalone Android, 100% offline!).
2. **[Contribute Translations via Pull Request](#2-contributing-translations-to-the-repository)** — improve existing **English (`EN`)** / **Bahasa Indonesia (`ID`)** wording, fix broken/wrapped UI labels, or submit a brand-new community language pack!

---

## 1. Bring Your Own Language Pack (BYOL)

You do **not** need to compile anything to run your own language (Spanish, Portuguese, French, German, Vietnamese, Thai, Russian, Arabic, Traditional Chinese, Korean, custom meme pack, or personal wording tweaks).

### Quick Start (3 Steps)

1. **Get the starter template:**
   - Download [`patches/custom-pack-template.json`](patches/custom-pack-template.json) — or click **Files → Template** inside `RuzaLangPatcher.exe`.
2. **Edit the `.json` file** in any text editor (VS Code, Notepad++, or any Android text editor).
3. **Load & install:**
   - **Windows PC / Android ADB (`RuzaLangPatcher.exe`):** Open **Files → Custom Language Pack → Load JSON**, select your `.json` file, then click **Install Patch (`<CODE>`)** on Home (or in the Android tab).
   - **Standalone Android (`RuzaLangPatcher.apk`):** Open **Files → Custom Language Pack → Load JSON**, pick your `.json` file from Downloads/Storage, then tap **Install Patch (`<CODE>`)** on Home.
   - To go back to built-in `EN` / `ID`, simply click **Clear** next to **Custom Language Pack** (or click **Rollback JP** to restore Original JP).

> [!TIP]
> **One file works on BOTH PC and Android!** Always use the canonical **PC `VStringPool` numeric IDs** (from `patches/full-en-3219598.json`) and **Static UI keys** (from `patches/static-ui-en-3219598.json`). Both the Windows app and Android APK automatically remap your PC IDs to Android IDs on the fly!

---

### Custom Language Pack JSON Format

#### Option A — Structured Pack (Recommended)

```json
{
  "meta": {
    "name": "Español (ES)",
    "code": "ES",
    "author": "YourName",
    "base": "en"
  },
  "vstring": {
    "1504449": "Equipar",
    "1562150": "Skills",
    "1496956": "Guardia",
    "1510370": "Evasión Perfecta",
    "1526899": "Bloqueo Perfecto: Contraataque"
  },
  "static_ui": {
    "15DC305A": "Equipar",
    "4C4E41E1": "Mejorar",
    "38CF16F2": "Confirmar",
    "3186527E": "Cancelar"
  },
  "builtin": {
    "confirm": "Confirmar",
    "cancel": "Cancelar"
  }
}
```

- **`meta.base`:** Controls what happens to strings you haven't translated yet:
  - `"en"` *(default)* — inherits built-in **English (`EN`)** for untranslated keys, then applies your overrides on top. Great for partial packs or wording tweaks!
  - `"id"` — inherits built-in **Bahasa Indonesia (`ID`)**, then applies your overrides on top.
  - `"jp"` — keeps **Original Japanese** for any key not listed in your pack.
- **`meta.code`:** Short uppercase badge (up to 8 characters, e.g., `ES`, `PT-BR`, `VI`, `TH`, `FR`, `RU`).
- **Comments:** Any key starting with `_` (such as `"_comment"` or `"_hint"`) is safely ignored by the patcher and validator.

#### Option B — Flat Dictionary Drop-In

You can also directly copy, edit, and load `patches/full-en-3219598.json` or `patches/static-ui-en-3219598.json` **without wrapping it in `meta`/`vstring`**! The patcher automatically classifies keys:
- Pure numeric keys (`"1504449"`) → `vstring`
- 8-hex keys (`"15DC305A"`, `"CB185163+1"`) → `static_ui`
- Other keys (`"confirm"`) → `builtin`

---

### Where to Find String IDs

All canonical dictionaries are human-readable JSON right inside [`patches/`](patches/):

| File | Coverage | Entries | Notes |
| :--- | :--- | :---: | :--- |
| [`patches/full-en-3219598.json`](patches/full-en-3219598.json) | Dialogue, quests, skills, items, settings (`EN`) | `74,055` | Canonical PC IDs (auto-remapped on Android) |
| [`patches/full-id-3219598.json`](patches/full-id-3219598.json) | Dialogue, quests, skills, items, settings (`ID`) | `74,055` | Indonesian reference |
| [`patches/static-ui-en-3219598.json`](patches/static-ui-en-3219598.json) | Prefab buttons, tabs, headers, notices (`EN`) | `3,135` | Works identically on PC & Android |
| [`patches/static-ui-id-3219598.json`](patches/static-ui-id-3219598.json) | Prefab buttons, tabs, headers, notices (`ID`) | `3,135` | Indonesian Static UI reference |

Search any English or Indonesian phrase in those JSON files to find its exact key!

---

## 2. Contributing Translations to the Repository

Pull Requests that fix awkward wording, align combat/in-game terminology, prevent UI wrapping, or add a new community language are always welcome!

### Translation Quality & Safety Rules

1. **Preserve every placeholder verbatim:** Never rename, reorder, or remove `{0}`, `{1}`, `{playerName}`, `%s`, `%d`, or `|` (used for gender/variant splits like `he|she`).
2. **Preserve rich-text tags cleanly:** Keep `<color=#...>...</color>`, `<b>...</b>`, `<i>...</i>`, and `<size=...>...</size>` properly paired and nested.
3. **Keep tight UI buttons concise:** In-game buttons, tabs, and pills have fixed widths. Prefer crisp RPG terms (`Equip`, `Skills`, `Rec.`, `Ascend`, `Guard`, `Perfect Guard`, `Perfect Parry`, `Perfect Dodge`) instead of long literal sentences that wrap mid-word.
4. **Stay within exact-size byte budgets:** Ruza-Lang preserves exact asset file sizes. Avoid bloating thousands of strings far beyond the original Japanese UTF-8 byte length (for `static_ui`, LZ4HC exact-size compression also needs compact, consistent phrasing).

### Validate Before Testing or Submitting a PR

Run the built-in zero-dependency validator (`Node.js 18+`):

```bash
node tools/validate-pack.mjs path/to/your-pack.json
```

It checks JSON syntax, key format, placeholder parity, Unity tag balance, tight UI wrap risks, and UTF-8 byte growth!

### Submitting Your Pull Request

- **Fixing existing `EN` / `ID` strings:**
  1. Fork the repository and edit `patches/full-en-3219598.json`, `patches/full-id-3219598.json`, `patches/static-ui-en-3219598.json`, or `patches/static-ui-id-3219598.json`.
  2. Run `node tools/validate-pack.mjs <edited-file.json>` and confirm `[PASS]`.
  3. Open a Pull Request describing the context (screenshots of in-game UI are super helpful!). Maintainers will repack `resources/*.bin.gz` and refresh SHA-256 verification hashes cleanly.
- **Sharing a new community language pack (e.g. `ES`, `PT-BR`, `VI`, `TH`, `FR`, `RU`):**
  1. Save your pack under `patches/community/<lang-code>.json` (e.g. `patches/community/es.json`) using the structured schema above.
  2. Validate it with `node tools/validate-pack.mjs patches/community/<lang-code>.json` and test it in-game via **Files → Custom Language Pack → Load JSON**!
  3. Open a Pull Request — players can immediately download and load your JSON pack on both PC and Android, and complete high-quality packs can be promoted to built-in languages!
