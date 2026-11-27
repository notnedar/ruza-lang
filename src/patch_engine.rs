use crate::crypto_lz4::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use sysinfo::{ProcessesToUpdate, System};

// Embedded verified payloads compiled directly into the standalone executable
pub const EMBED_MMAP_KEY: &[u8] = include_bytes!("../resources/mmap_key_4096.bin");

const EMBED_VSTRING_EN_GZ: &[u8] = include_bytes!("../resources/vstring_en.bin.gz");
const EMBED_STATIC_UI_EN_GZ: &[u8] = include_bytes!("../resources/static_ui_en.bin.gz");
const EMBED_BUILTIN_EN_GZ: &[u8] = include_bytes!("../resources/builtin_en.bin.gz");
const EMBED_RESOURCES_DIFF_EN_GZ: &[u8] = include_bytes!("../resources/resources_diff.bin.gz");

const EMBED_VSTRING_ID_GZ: &[u8] = include_bytes!("../resources/vstring_id.bin.gz");
const EMBED_STATIC_UI_ID_GZ: &[u8] = include_bytes!("../resources/static_ui_id.bin.gz");
const EMBED_BUILTIN_ID_GZ: &[u8] = include_bytes!("../resources/builtin_id.bin.gz");
const EMBED_RESOURCES_DIFF_ID_GZ: &[u8] = include_bytes!("../resources/resources_diff_id.bin.gz");

const EMBED_NEWS_EN_GZ: &[u8] = include_bytes!("../resources/news_en.bin.gz");
const EMBED_NEWS_ID_GZ: &[u8] = include_bytes!("../resources/news_id.bin.gz");

pub const VSTRING_STRINGS: usize = 76180;
pub const STATIC_UI_STRINGS: usize = 3135;
pub const STATIC_PKG_STRINGS: usize = 113;
pub const RESOURCES_STRINGS: usize = 358;
pub const TOTAL_STRINGS: usize = VSTRING_STRINGS + STATIC_UI_STRINGS + STATIC_PKG_STRINGS + RESOURCES_STRINGS; // 79,786
pub const ANDROID_TOTAL_STRINGS: usize = VSTRING_STRINGS + STATIC_UI_STRINGS; // 79,315 (writable external .res bundles; zero APK re-signing)

// Verified client build metadata for version sync checks
pub const EXPECTED_CLIENT_VERSION: &str = "1.0.2";
pub const EXPECTED_RES_REVISION: &str = "3273853";
pub const EXPECTED_BUILD_GUID: &str = "6310abcfe75841bcac713c29234fe986";

// Verified Android client metadata (r3273851 / r3219596+, v1.0.0 — universal hotfix-proof engine)
pub const ANDROID_PACKAGE_ID: &str = "com.Yostar.AzurpromiliaJP";
pub const ANDROID_EXPECTED_VERSION: &str = "1.0.0";
pub const ANDROID_EXPECTED_RES_REV: &str = "3273851";

// Constant Unity CAB directory name for ast_staticTextKeyConfig across PC & Android builds
pub const STATIC_UI_CAB_NAME: &str = "CAB-d373fa6fedc5b221683a433ac01d8eb0";
pub const KNOWN_PC_STATIC_UI_NAMES: &[&str] = &["h3y7ujukwn9qbhoqhzkkg.r", "yr49bxop0f3reujfsmyfca.r"];
pub const KNOWN_ANDROID_STATIC_UI_NAMES: &[&str] = &["nibdvaxcnfh3en5bnvhtw.r"];

pub const RAW_BUNDLE_REL_DIR: &str = r"AzurPromiliaJP_Data\.res\raw_base_package\BundleFiles";
pub const DEFAULT_BUNDLE_REL_DIR: &str = r"AzurPromiliaJP_Data\.res\default_package\BundleFiles";
pub const STATIC_PKG_REL: &str = r"AzurPromiliaJP_Data\StreamingAssets\.res\static_package\6pq4a7hlp7zidu91dvvww.r";
pub const RESOURCES_REL: &str = r"AzurPromiliaJP_Data\resources.assets";

// Known Original JP SHA-256 hashes (r3219598 & r3273853 PC + r3219596 Android)
pub const VSTRING_ORIG_SHAS: &[&str] = &[
    "99B8129D49586D26745630A76423609123DDCB1A7AAD3E0A971886B13336A878", // PC r3219598
];
pub const STATIC_UI_ORIG_SHAS: &[&str] = &[
    "701F04842F5156DA177BD4946A1CF9B7C055F2A614003C2C13C8567FA14E0FC3", // PC r3219598
];
pub const ANDROID_VSTRING_ORIG_SHAS: &[&str] = &[
    "C6BA69F2403102A9D4DBF2B3AB079ABBE7DE25896ABDB3969DD5B7988A2DB013", // Android r3219596
];
pub const ANDROID_STATIC_UI_ORIG_SHAS: &[&str] = &[
    "53A36C59B4ABABF9457ED37B195092DCAFD491C2D95D9BA75A61998C0138D22A", // Android r3219596
];

// Known legacy/current patched SHA-256 hashes (in addition to dynamic _ruza_state.json tracking!)
pub const VSTRING_KNOWN_EN_SHAS: &[&str] = &[
    "81B770280B2C8C307B7CD7A3CC038207B392A6256D34514FAB8ACE410A82F57B", // v0.9.0 PC r3219598
    "9C4283483E66F334929C92E620BB1A7345E83CBF10DF29DF4BD17CBB20687F05", // v0.9.0 PC r3273853
    "61A6FCC2A32950381D5468AC5D1E6C02651D6B89BC846E220EC266395EDE1E7E", // v0.9.1 PC r3219598
    "D3749817194F947BE336AE855300EF9AE8CB59CCA6EC2B18D85AA93A006589D8", // v0.9.1 PC r3273853
];
pub const VSTRING_KNOWN_ID_SHAS: &[&str] = &[
    "ED57A17D9C884CE436C81533D1A772C30AEE3FBCD85093C694B6973097BB904F", // v0.9.0 PC r3219598
    "5B0C27CBBA77FD7B9BCAC0D4A263177FCAEFE7FAEBB20A1516BA802028183D4A", // v0.9.0 PC r3273853
    "25B6E35D1CDC49E2F49315DD98C7538692082C350B54650CD5A9EC68919AE524", // v0.9.1 PC r3219598
    "2FC141815581A91928C2B8E730666D7EC6A831E0F7B3C59F0B85C8D78BF57433", // v0.9.1 PC r3273853
];
pub const STATIC_UI_KNOWN_EN_SHAS: &[&str] = &[
    "8679E44FF0EA0493F464201DD26AC9D4185935CB3A98F99649559066C492BFDA", // v0.9.0 PC r3219598
    "B0CF69FFF31ACB0D3D9D6B9A267D0C16B15A64FBE9E27E9FA93CF7468DDDF11C", // v0.9.0 PC r3273853
    "9F0270ACF3A046A441AE25BEF3CC1CE039EBA0DE22ED179808FECD9875ACF26C", // v0.9.1 PC r3219598
    "7900C693A5C922A52B376AB3B62D03DA18E5C03E446B812D1D49CE3DA3FD6683", // v0.9.1 PC r3273853
];
pub const STATIC_UI_KNOWN_ID_SHAS: &[&str] = &[
    "E1E6E9A015DCC4E70994D81E8F2FE3ED8437805F58F61455A470991FADC05772", // v0.8.x PC r3219598
    "502196E9F8BA4946EF0E0611EA50136482C103D712D218E674F8B015F9CE7DA9", // v0.9.0 PC r3219598
    "6A64B6FCEAA1C42264F966A1870F4A36E8B1044D93EF1DFDCA046F0617E7FE5F", // v0.9.0 PC r3273853
    "5A6B51F93E398EB3B01FF68D52FA6C122B8D70852342FAC47B5095FAFFA18483", // v0.9.1 PC r3219598
    "DE9E6053428625B8CC78E4C126EB8BDDB41A4B5E5D895FD7FE14C5A37CE59336", // v0.9.1 PC r3273853
];
pub const ANDROID_VSTRING_KNOWN_EN_SHAS: &[&str] = &[
    "0739151C287863A4CD45150CC85DE3DDFDD3A7587FB159437F3F5FEED513A4E5", // v0.8.x Android r3219596
    "3216E4E2A56F2BC36EAA2FFF79B203C854FA834A93A97C2CA218B40E909FF5F1", // v0.9.0 Android r3219596
    "2124FBF99624837DD6C36F0712EDB573ACFDE62DAC304FEF87678E6B91EDE1FF", // v0.9.1 Android r3219596
];
pub const ANDROID_VSTRING_KNOWN_ID_SHAS: &[&str] = &[
    "BD9A20B6BE6F9228E12F435F72374386F8C15D270E30B442927401A2EB36A0AF", // v0.8.x Android r3219596
    "EBC5EE0155910A58D3B62AB978A886CCD1BA4FA68F4CE929B7763E32454531DE", // v0.9.0 Android r3219596
    "43DA80F93DACA269313BCBF0B6D14BC3685F25AE479806419DFF848E24AD7150", // v0.9.1 Android r3219596
];
pub const ANDROID_STATIC_UI_KNOWN_EN_SHAS: &[&str] = &[
    "2A4428AE21009DB19621A123B1A2561ED1F83A685B4C9160B94BA74CD50E8DAC", // v0.9.0 Android r3219596
    "EB18599A3513CDF558F4716E0B70DEA233CAC9076D50BA6EFEB0D7958C87F01B", // v0.9.1 Android r3219596
];
pub const ANDROID_STATIC_UI_KNOWN_ID_SHAS: &[&str] = &[
    "0017773EB20E5B56D3E02179A9F94A6FBC862B48CFD9FF7CF443CCAE9E6A64B2", // v0.8.x Android r3219596
    "60F0B86F98FCA84D46AE1AD7531510320812972D5371F80FC358860B660BC308", // v0.9.0 Android r3219596
    "10939AD972515B9A6DF118AC5D7ADA99BBA7F0C0FBF54FCAE79698987B7EE59B", // v0.9.1 Android r3219596
];

pub const STATIC_PKG_ORIG_SHA: &str = "C7C80B98B263F161583A2AB52A5B7039636ACBABCAC8C365F9F37224D11BC4B3";
pub const RESOURCES_ORIG_SHA: &str = "DF93CE7192D6A46B33350204349E66A45E410124E3057224AD1A17E6F24B115B";

pub const STATIC_PKG_EN_SHA: &str = "7FEB394BD2FEAC1F8FC862A1254CD954EDC1D49544DE285D3E6D9C914755F33E";
pub const RESOURCES_EN_SHA: &str = "C95E1A2C5C5977E105022B49266D3E2E32173A0E1F4E75BD807C7A93A9299883";

pub const STATIC_PKG_ID_SHA: &str = "3EBE582B61D995108E1DA1F333D102B6218A5C9D33E03C931CB7EEEE585DC1FC";
pub const RESOURCES_ID_SHA: &str = "85FADFAAC9708C845803FA438C5C2AA634A798FAE48405C9FC40953B4A7772A2";

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BundlePatchState {
    pub file_name: String,
    pub sub_offset: usize,
    pub lang: String,        // "patched_en" | "patched_id" | "patched_custom" | "original"
    pub custom_code: String,
    pub orig_sha: String,
    pub patched_sha: String,
    pub matched_count: usize,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PatchStateFile {
    pub schema: u32,
    pub vstring: BundlePatchState,
    pub static_ui: BundlePatchState,
    pub static_pkg: BundlePatchState,
    pub resources: BundlePatchState,
}

pub fn load_patch_state(bak_dir: &Path) -> PatchStateFile {
    let p = bak_dir.join("_ruza_state.json");
    let Ok(txt) = fs::read_to_string(&p) else {
        return PatchStateFile { schema: 1, ..Default::default() };
    };
    serde_json::from_str::<PatchStateFile>(&txt).unwrap_or(PatchStateFile { schema: 1, ..Default::default() })
}

pub fn save_patch_state(bak_dir: &Path, state: &PatchStateFile) {
    let _ = fs::create_dir_all(bak_dir);
    let mut s = state.clone();
    s.schema = 1;
    if let Ok(txt) = serde_json::to_string_pretty(&s) {
        let _ = fs::write(bak_dir.join("_ruza_state.json"), txt);
    }
}

pub fn state_tag_for(lang: PatchLang, custom: Option<&CustomPack>) -> String {
    if custom.is_some() {
        "patched_custom".to_string()
    } else {
        match lang {
            PatchLang::En => "patched_en".to_string(),
            PatchLang::Id => "patched_id".to_string(),
        }
    }
}

pub fn sha_in_list(sha: &str, list: &[&str]) -> bool {
    list.iter().any(|h| !h.is_empty() && sha.eq_ignore_ascii_case(h))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PatchLang {
    En,
    Id,
}

impl PatchLang {
    pub fn from_opt(s: Option<&str>) -> Self {
        match s.map(|v| v.to_ascii_lowercase()).as_deref() {
            Some("id") => PatchLang::Id,
            _ => PatchLang::En,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PatchLang::En => "English (EN)",
            PatchLang::Id => "Bahasa Indonesia (ID)",
        }
    }

    pub fn short_code(self) -> &'static str {
        match self {
            PatchLang::En => "EN",
            PatchLang::Id => "ID",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomBase {
    En,
    Id,
    Jp,
}

impl CustomBase {
    pub fn from_str_opt(s: Option<&str>) -> Self {
        match s.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("id") => CustomBase::Id,
            Some("jp") | Some("none") | Some("orig") | Some("original") => CustomBase::Jp,
            _ => CustomBase::En,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CustomBase::En => "en",
            CustomBase::Id => "id",
            CustomBase::Jp => "jp",
        }
    }
}

#[derive(Clone, Serialize)]
pub struct CustomPackMeta {
    pub path: String,
    pub file_name: String,
    pub name: String,
    pub code: String,
    pub author: String,
    pub base: String, // "en" | "id" | "jp"
    pub vstring_count: usize,
    pub static_ui_count: usize,
    pub builtin_count: usize,
    pub total_count: usize,
}

#[derive(Clone)]
pub struct CustomPack {
    pub meta: CustomPackMeta,
    pub base: CustomBase,
    pub vstring: HashMap<usize, Vec<u8>>,
    pub static_ui: HashMap<String, String>,
    pub builtin: HashMap<String, String>,
}

fn is_static_ui_key(k: &str) -> bool {
    let mut parts = k.split('+');
    let Some(hex_part) = parts.next() else {
        return false;
    };
    if hex_part.len() != 8 || !hex_part.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    match parts.next() {
        None => true,
        Some(suf) => parts.next().is_none() && !suf.is_empty() && suf.bytes().all(|b| b.is_ascii_digit()),
    }
}

fn sanitize_pack_code(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(8)
        .collect();
    if cleaned.is_empty() {
        "CUSTOM".to_string()
    } else {
        cleaned.to_ascii_uppercase()
    }
}

pub fn parse_custom_pack_str(raw: &str, source_path: &str, file_name: &str) -> Result<CustomPack, String> {
    let cleaned_raw = raw.strip_prefix('\u{FEFF}').unwrap_or(raw);
    let root: serde_json::Value = serde_json::from_str(cleaned_raw)
        .map_err(|e| format!("Invalid custom pack JSON: {}", e))?;
    let obj = root
        .as_object()
        .ok_or_else(|| "Custom pack JSON root must be an object { ... }.".to_string())?;

    let default_name = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Custom Pack")
        .to_string();

    let meta_obj = obj.get("meta").and_then(|v| v.as_object());
    let name = meta_obj
        .and_then(|m| m.get("name").and_then(|v| v.as_str()))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or(default_name);
    let code = sanitize_pack_code(
        meta_obj
            .and_then(|m| m.get("code").and_then(|v| v.as_str()))
            .unwrap_or("CUSTOM"),
    );
    let author = meta_obj
        .and_then(|m| m.get("author").and_then(|v| v.as_str()))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let base = CustomBase::from_str_opt(
        meta_obj.and_then(|m| m.get("base").and_then(|v| v.as_str())),
    );

    let mut vstring: HashMap<usize, Vec<u8>> = HashMap::new();
    let mut static_ui: HashMap<String, String> = HashMap::new();
    let mut builtin: HashMap<String, String> = HashMap::new();

    let has_structured = obj.get("vstring").and_then(|v| v.as_object()).is_some()
        || obj.get("static_ui").and_then(|v| v.as_object()).is_some()
        || obj.get("builtin").and_then(|v| v.as_object()).is_some();

    if has_structured {
        if let Some(vs_obj) = obj.get("vstring").and_then(|v| v.as_object()) {
            for (k, val) in vs_obj {
                if k.starts_with('_') {
                    continue;
                }
                let Some(s) = val.as_str() else {
                    return Err(format!("Custom pack vstring[\"{}\"] must be a string.", k));
                };
                let id = k
                    .parse::<usize>()
                    .map_err(|_| format!("Custom pack vstring key \"{}\" must be a numeric ID.", k))?;
                vstring.insert(id, s.as_bytes().to_vec());
            }
        }
        if let Some(ui_obj) = obj.get("static_ui").and_then(|v| v.as_object()) {
            for (k, val) in ui_obj {
                if k.starts_with('_') {
                    continue;
                }
                let Some(s) = val.as_str() else {
                    return Err(format!("Custom pack static_ui[\"{}\"] must be a string.", k));
                };
                static_ui.insert(k.clone(), s.to_string());
            }
        }
        if let Some(bi_obj) = obj.get("builtin").and_then(|v| v.as_object()) {
            for (k, val) in bi_obj {
                if k.starts_with('_') {
                    continue;
                }
                let Some(s) = val.as_str() else {
                    return Err(format!("Custom pack builtin[\"{}\"] must be a string.", k));
                };
                builtin.insert(k.clone(), s.to_string());
            }
        }
    } else {
        // Raw dictionary drop-in (e.g. direct copy/edit of patches/full-en-3219598.json or static-ui-en-3219598.json)
        for (k, val) in obj {
            if k == "meta" || k.starts_with('_') {
                continue;
            }
            let Some(s) = val.as_str() else {
                continue;
            };
            if is_static_ui_key(k) {
                static_ui.insert(k.clone(), s.to_string());
            } else if !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()) {
                if let Ok(id) = k.parse::<usize>() {
                    vstring.insert(id, s.as_bytes().to_vec());
                }
            } else {
                builtin.insert(k.clone(), s.to_string());
            }
        }
    }

    let total_count = vstring.len() + static_ui.len() + builtin.len();
    if total_count == 0 {
        return Err(
            "Custom pack contains 0 translation entries. Add entries under \"vstring\", \"static_ui\", or \"builtin\" (or provide a flat ID->text JSON dictionary)."
                .to_string(),
        );
    }

    let meta = CustomPackMeta {
        path: source_path.to_string(),
        file_name: file_name.to_string(),
        name,
        code,
        author,
        base: base.as_str().to_string(),
        vstring_count: vstring.len(),
        static_ui_count: static_ui.len(),
        builtin_count: builtin.len(),
        total_count,
    };

    Ok(CustomPack {
        meta,
        base,
        vstring,
        static_ui,
        builtin,
    })
}

pub fn load_custom_pack(path: &Path) -> Result<CustomPack, String> {
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("Failed reading custom pack {:?}: {}", path, e))?;
    let fname = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("custom-pack.json");
    parse_custom_pack_str(&raw, &path.to_string_lossy(), fname)
}

pub const CUSTOM_PACK_TEMPLATE_JSON: &str = r#"{
  "_comment": "Ruza-Lang Bring-Your-Own Language Pack Template (PC & Android). Set meta.base to 'en', 'id', or 'jp' (untranslated keys inherit from base). Keys starting with '_' are ignored.",
  "meta": {
    "name": "My Custom Language",
    "code": "CUSTOM",
    "author": "YourName",
    "base": "en"
  },
  "vstring": {
    "_hint": "Canonical PC VStringPool numeric IDs (auto-remapped for Android!)",
    "1504449": "Equip",
    "1562150": "Skills",
    "1496956": "Guard",
    "1510370": "Perfect Dodge",
    "1510372": "Dodge Counter",
    "1510387": "Parry Window",
    "1526899": "Perfect Guard: Counter"
  },
  "static_ui": {
    "_hint": "Static UI KeyConfig prefab labels (works on both PC & Android)",
    "15DC305A": "Equip",
    "4C4E41E1": "Upgrade",
    "5A304CCA": "Compare",
    "106FC6A7": "Ascend",
    "2F7A37CE": "Party",
    "4EFE9AA0": "Bag",
    "5F411223": "Back",
    "38CF16F2": "Confirm",
    "3186527E": "Cancel"
  },
  "builtin": {
    "_hint": "PC startup & updater messages (optional, ignored on Android)",
    "alert_title": "Alert",
    "confirm": "Confirm",
    "cancel": "Cancel",
    "retry": "Retry",
    "quit": "Quit",
    "initializing": "Initializing…",
    "checking_update": "Checking update…"
  }
}
"#;

pub fn export_custom_pack_template(dst: &Path) -> Result<(), String> {
    fs::write(dst, CUSTOM_PACK_TEMPLATE_JSON)
        .map_err(|e| format!("Failed writing template to {:?}: {}", dst, e))
}

#[derive(Clone, Serialize)]
pub struct ProcessInfo {
    pub name: String,
    pub pid: u32,
}

#[derive(Clone, Serialize)]
pub struct TargetStatus {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub rel_path: String,
    pub backup_name: String,
    pub string_count: usize,
    pub state: String, // "patched_en" | "patched_id" | "original" | "modified" | "missing"
    pub has_backup: bool,
    pub sha256: String,
}

#[derive(Clone, Serialize)]
pub struct ClientVersionInfo {
    pub detected_version: String,
    pub expected_version: String,
    pub detected_res_rev: String,
    pub expected_res_rev: String,
    pub detected_build_guid: String,
    pub expected_build_guid: String,
    pub status: String, // "synced" | "mismatch"
    pub reasons: Vec<String>,
}

#[derive(Clone, Copy)]
pub enum LogLevel {
    Info,
    Ok,
    Warn,
    Error,
    Diff,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Info => "info",
            LogLevel::Ok => "ok",
            LogLevel::Warn => "warn",
            LogLevel::Error => "err",
            LogLevel::Diff => "diff",
        }
    }
}

pub fn find_running_game_processes() -> Vec<ProcessInfo> {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let watched = ["azurpromiliajp.exe", "azupro_jp_gamelauncher.exe"];
    let mut out = Vec::new();
    for (pid, proc_) in sys.processes() {
        let name = proc_.name().to_string_lossy().to_string();
        let lower = name.to_ascii_lowercase();
        if watched.iter().any(|w| lower == *w) {
            out.push(ProcessInfo {
                name,
                pid: pid.as_u32(),
            });
        }
    }
    out
}

pub fn is_valid_game_folder(dir: &Path) -> bool {
    dir.join("AzurPromiliaJP.exe").is_file() && dir.join("AzurPromiliaJP_Data").is_dir()
}

pub fn auto_detect_game_folder() -> String {
    let mut candidates: Vec<PathBuf> = vec![
        PathBuf::from(r"E:\Program Files (x86)\YostarGames\AZUPRO_JP"),
        PathBuf::from(r"C:\Program Files (x86)\YostarGames\AZUPRO_JP"),
        PathBuf::from(r"D:\Program Files (x86)\YostarGames\AZUPRO_JP"),
        PathBuf::from(r"F:\Program Files (x86)\YostarGames\AZUPRO_JP"),
        PathBuf::from(r"C:\YostarGames\AZUPRO_JP"),
        PathBuf::from(r"D:\YostarGames\AZUPRO_JP"),
        PathBuf::from(r"E:\YostarGames\AZUPRO_JP"),
        PathBuf::from(r"D:\Games\AZUPRO_JP"),
        PathBuf::from(r"E:\Games\AZUPRO_JP"),
    ];
    for drive in b'C'..=b'Z' {
        let root = format!("{}:\\", drive as char);
        if Path::new(&root).exists() {
            candidates.push(PathBuf::from(&root).join(r"Program Files (x86)\YostarGames\AZUPRO_JP"));
            candidates.push(PathBuf::from(&root).join(r"Program Files\YostarGames\AZUPRO_JP"));
            candidates.push(PathBuf::from(&root).join(r"YostarGames\AZUPRO_JP"));
        }
    }
    for c in candidates {
        if is_valid_game_folder(&c) {
            return c.to_string_lossy().to_string();
        }
    }
    String::new()
}

#[allow(dead_code)]
fn sha_matches(sha: &str, expected: &str) -> bool {
    !expected.is_empty() && sha.eq_ignore_ascii_case(expected)
}

fn read_client_version(game_dir: &Path) -> String {
    // 1. Primary: game-launcher-config.json ("version": "1.0.2")
    let cfg = game_dir.join("game-launcher-config.json");
    if let Ok(text) = fs::read_to_string(&cfg) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(v) = val.get("version").and_then(|x| x.as_str()) {
                let trimmed = v.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
    }
    // 2. Fallback: header of manifest.json (read only first 1 KB)
    let man = game_dir.join("manifest.json");
    if let Ok(mut f) = fs::File::open(&man) {
        use std::io::Read;
        let mut buf = [0u8; 1024];
        if let Ok(n) = f.read(&mut buf) {
            let head = String::from_utf8_lossy(&buf[..n]);
            if let Some(idx) = head.find("\"version\"") {
                let rest = &head[idx + 9..];
                if let Some(colon) = rest.find(':') {
                    let after = rest[colon + 1..].trim_start();
                    if let Some(stripped) = after.strip_prefix('"') {
                        if let Some(endq) = stripped.find('"') {
                            let v = stripped[..endq].trim();
                            if !v.is_empty() {
                                return v.to_string();
                            }
                        }
                    }
                }
            }
        }
    }
    String::new()
}

fn read_res_revision(game_dir: &Path) -> String {
    let dirs = [
        game_dir.join(r"AzurPromiliaJP_Data\.res\raw_base_package\ManifestFiles"),
        game_dir.join(r"AzurPromiliaJP_Data\.res\default_package\ManifestFiles"),
    ];
    let mut best_num: Option<u64> = None;
    let mut best_str = String::new();
    for d in dirs {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(stem) = name.strip_suffix(".bytes") else { continue };
            if !stem.contains("_package_") {
                continue;
            }
            if let Some(rev) = stem.rsplit('_').next() {
                if !rev.is_empty() && rev.bytes().all(|b| b.is_ascii_digit()) {
                    if let Ok(num) = rev.parse::<u64>() {
                        if best_num.map(|b| num > b).unwrap_or(true) {
                            best_num = Some(num);
                            best_str = rev.to_string();
                        }
                    }
                }
            }
        }
    }
    best_str
}

fn read_build_guid(game_dir: &Path) -> String {
    let boot = game_dir.join(r"AzurPromiliaJP_Data\boot.config");
    if let Ok(text) = fs::read_to_string(&boot) {
        for line in text.lines() {
            if let Some(v) = line.trim().strip_prefix("build-guid=") {
                let g = v.trim();
                if !g.is_empty() {
                    return g.to_string();
                }
            }
        }
    }
    String::new()
}

pub fn detect_client_version(game_dir: &Path, targets: &[TargetStatus]) -> ClientVersionInfo {
    let detected_version = read_client_version(game_dir);
    let detected_res_rev = read_res_revision(game_dir);
    let detected_build_guid = read_build_guid(game_dir);

    let mut reasons = Vec::new();
    if !detected_version.is_empty() && detected_version != EXPECTED_CLIENT_VERSION {
        reasons.push(format!(
            "Client version {} != {}",
            detected_version, EXPECTED_CLIENT_VERSION
        ));
    }
    if !detected_build_guid.is_empty()
        && !detected_build_guid.eq_ignore_ascii_case(EXPECTED_BUILD_GUID)
    {
        reasons.push(format!(
            "Build GUID {}... differs from verified build",
            &detected_build_guid[..8.min(detected_build_guid.len())]
        ));
    }

    let missing_core = targets
        .iter()
        .filter(|t| (t.id == "vstring" || t.id == "static_ui") && t.state == "missing")
        .count();
    let missing_cnt = targets.iter().filter(|t| t.state == "missing").count();
    let modified_cnt = targets
        .iter()
        .filter(|t| t.state == "modified" && !t.has_backup)
        .count();

    // Hotfix-proof Smart Version Sync: if both core bundles (VStringPool + Static UI) were
    // dynamically discovered & validated, resource hotfix revisions (e.g. r3219598 / r3273853 / r3300000+)
    // stay Synced automatically without false mismatch popups!
    if missing_core > 0 && !detected_res_rev.is_empty() && detected_res_rev != EXPECTED_RES_REVISION {
        reasons.push(format!(
            "Resource revision r{} != r{}",
            detected_res_rev, EXPECTED_RES_REVISION
        ));
    }
    if modified_cnt > 0 {
        reasons.push(format!(
            "{} package file(s) differ from verified SHA-256 hashes",
            modified_cnt
        ));
    }
    if missing_cnt > 0 {
        reasons.push(format!("{} package file(s) missing", missing_cnt));
    }

    let status = if reasons.is_empty() {
        "synced".to_string()
    } else {
        "mismatch".to_string()
    };

    ClientVersionInfo {
        detected_version,
        expected_version: EXPECTED_CLIENT_VERSION.to_string(),
        detected_res_rev,
        expected_res_rev: EXPECTED_RES_REVISION.to_string(),
        detected_build_guid,
        expected_build_guid: EXPECTED_BUILD_GUID.to_string(),
        status,
        reasons,
    }
}

pub struct DiscoveredVString {
    pub file_name: String,
    pub rel_path: String,
    pub full_path: PathBuf,
    pub entry_count: usize,
}

pub struct DiscoveredStaticUi {
    pub file_name: String,
    pub rel_path: String,
    pub full_path: PathBuf,
    pub sub_offset: usize,
    pub sub_size: usize,
}

fn file_mtime_secs(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn probe_vstring_file(path: &Path) -> Option<usize> {
    let meta = fs::metadata(path).ok()?;
    let fsize = meta.len() as usize;
    if fsize < 50_000_000 {
        return None;
    }
    let mut f = File::open(path).ok()?;
    f.seek(SeekFrom::Start(4120)).ok()?;
    let mut h1 = [0u8; 24];
    f.read_exact(&mut h1).ok()?;
    xor_mmap_in_place(&mut h1, 4120, EMBED_MMAP_KEY);
    let hash_bytes = read_u32_le(&h1, 0) as usize;
    let hash_start = read_u32_le(&h1, 4) as usize;
    if hash_start != 4120 {
        return None;
    }
    let str_sec_off = 4120usize.checked_add(hash_bytes)?;
    if str_sec_off + 20 > fsize {
        return None;
    }
    f.seek(SeekFrom::Start(str_sec_off as u64)).ok()?;
    let mut sh = [0u8; 20];
    f.read_exact(&mut sh).ok()?;
    xor_mmap_in_place(&mut sh, str_sec_off, EMBED_MMAP_KEY);
    let count = read_u32_le(&sh, 16) as usize;
    if count < 500_000 || str_sec_off + 20 + count.saturating_mul(4) >= fsize {
        return None;
    }
    Some(count)
}

pub fn discover_pc_vstring(game_dir: &Path) -> Option<DiscoveredVString> {
    let dir = game_dir.join(RAW_BUNDLE_REL_DIR);
    let rd = fs::read_dir(&dir).ok()?;
    let mut best: Option<(usize, u64, String, PathBuf)> = None;
    for ent in rd.flatten() {
        let path = ent.path();
        if !path.is_file() {
            continue;
        }
        let fname = ent.file_name().to_string_lossy().to_string();
        if !fname.ends_with(".r") {
            continue;
        }
        let Some(cnt) = probe_vstring_file(&path) else {
            continue;
        };
        let mt = file_mtime_secs(&path);
        let better = match &best {
            None => true,
            Some((bc, bm, _, _)) => cnt > *bc || (cnt == *bc && mt > *bm),
        };
        if better {
            best = Some((cnt, mt, fname, path));
        }
    }
    let (entry_count, _, file_name, full_path) = best?;
    let rel_path = format!(r"{}\{}", RAW_BUNDLE_REL_DIR, file_name);
    Some(DiscoveredVString {
        file_name,
        rel_path,
        full_path,
        entry_count,
    })
}

pub fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

pub fn find_static_ui_subbundle_in_bytes(buf: &[u8]) -> Option<(usize, usize)> {
    const MAGIC: &[u8; 8] = b"UnityFS\0";
    if buf.len() < 128 {
        return None;
    }
    let mut fallback_candidates: Vec<usize> = Vec::new();
    let mut pos = 0usize;
    while pos + 8 <= buf.len() {
        let Some(rel) = find_bytes(&buf[pos..], MAGIC) else {
            break;
        };
        let off = pos + rel;
        pos = off + 8;
        if let Ok(bundle) = UnityCnBundle::parse(buf, off) {
            if bundle.dirs.len() == 1 && bundle.dirs[0].name == STATIC_UI_CAB_NAME {
                return Some((off, bundle.fsize));
            }
            if bundle.dirs.len() == 1
                && bundle.fsize >= 30_000
                && bundle.fsize <= 500_000
                && bundle.dirs[0].size >= 80_000
                && bundle.dirs[0].size <= 450_000
            {
                fallback_candidates.push(off);
            }
        }
    }
    // Fallback if a future major client changes the Unity CAB hash: verify ast_staticTextKeyConfig inside decrypted SF
    for off in fallback_candidates {
        if let Ok(bundle) = UnityCnBundle::parse(buf, off) {
            if let Ok(sf) = bundle.read_all() {
                if find_bytes(&sf, b"ast_staticTextKeyConfig").is_some() {
                    return Some((off, bundle.fsize));
                }
            }
        }
    }
    None
}

pub fn discover_pc_static_ui(game_dir: &Path, cached_name: Option<&str>) -> Option<DiscoveredStaticUi> {
    let dir = game_dir.join(DEFAULT_BUNDLE_REL_DIR);
    if !dir.is_dir() {
        return None;
    }
    let mut checked = std::collections::HashSet::new();

    // Tier 1: cached filename from _ruza_state.json + known filenames (< 5ms)
    let mut fast_names: Vec<String> = Vec::new();
    if let Some(c) = cached_name {
        if !c.is_empty() {
            fast_names.push(c.to_string());
        }
    }
    for &k in KNOWN_PC_STATIC_UI_NAMES {
        fast_names.push(k.to_string());
    }
    for fname in fast_names {
        if !checked.insert(fname.clone()) {
            continue;
        }
        let p = dir.join(&fname);
        if !p.is_file() {
            continue;
        }
        if let Ok(buf) = fs::read(&p) {
            if let Some((sub_offset, sub_size)) = find_static_ui_subbundle_in_bytes(&buf) {
                return Some(DiscoveredStaticUi {
                    rel_path: format!(r"{}\{}", DEFAULT_BUNDLE_REL_DIR, fname),
                    file_name: fname,
                    full_path: p,
                    sub_offset,
                    sub_size,
                });
            }
        }
    }

    // Tier 2/3: scan default_package/BundleFiles/*.r (3-10 MB newest-first, then remaining 100 KB-30 MB)
    let Ok(rd) = fs::read_dir(&dir) else {
        return None;
    };
    let mut cands: Vec<(bool, u64, String, PathBuf)> = Vec::new();
    for ent in rd.flatten() {
        let p = ent.path();
        if !p.is_file() {
            continue;
        }
        let fname = ent.file_name().to_string_lossy().to_string();
        if !fname.ends_with(".r") || checked.contains(&fname) {
            continue;
        }
        let Ok(meta) = fs::metadata(&p) else { continue };
        let sz = meta.len();
        if !(100_000..=30_000_000).contains(&sz) {
            continue;
        }
        let in_sweet_spot = (2_500_000..=10_000_000).contains(&sz);
        let mt = file_mtime_secs(&p);
        cands.push((!in_sweet_spot, u64::MAX - mt, fname, p));
    }
    cands.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    for (_, _, fname, p) in cands {
        if let Ok(buf) = fs::read(&p) {
            if let Some((sub_offset, sub_size)) = find_static_ui_subbundle_in_bytes(&buf) {
                return Some(DiscoveredStaticUi {
                    rel_path: format!(r"{}\{}", DEFAULT_BUNDLE_REL_DIR, fname),
                    file_name: fname,
                    full_path: p,
                    sub_offset,
                    sub_size,
                });
            }
        }
    }
    None
}

pub fn classify_bundle_state(
    file_name: &str,
    cur_sha: &str,
    bak_path: &Path,
    state_entry: &BundlePatchState,
    known_orig: &[&str],
    known_en: &[&str],
    known_id: &[&str],
    is_dynamic_bundle: bool,
) -> String {
    if state_entry.file_name == file_name
        && !state_entry.patched_sha.is_empty()
        && cur_sha.eq_ignore_ascii_case(&state_entry.patched_sha)
        && !state_entry.lang.is_empty()
    {
        return state_entry.lang.clone();
    }
    if sha_in_list(cur_sha, known_id) {
        return "patched_id".into();
    }
    if sha_in_list(cur_sha, known_en) {
        return "patched_en".into();
    }
    if sha_in_list(cur_sha, known_orig) {
        return "original".into();
    }
    if state_entry.file_name == file_name
        && !state_entry.orig_sha.is_empty()
        && cur_sha.eq_ignore_ascii_case(&state_entry.orig_sha)
    {
        return "original".into();
    }
    if bak_path.is_file() {
        if let Ok(bak_sha) = sha256_file(bak_path) {
            if cur_sha.eq_ignore_ascii_case(&bak_sha) {
                return "original".into();
            }
        }
        return "modified".into();
    }
    // Newly discovered hotfix bundle (no backup saved yet for this filename, and doesn't match any known patched SHA)
    if is_dynamic_bundle {
        "original".into()
    } else {
        "modified".into()
    }
}

pub fn scan_game_folder<F>(game_dir: &Path, mut log: F) -> Vec<TargetStatus>
where
    F: FnMut(&str, LogLevel),
{
    let bak_dir = game_dir.join("_ENPatchBackup");
    let state_file = load_patch_state(&bak_dir);

    let disc_vs = discover_pc_vstring(game_dir);
    let cached_ui = if state_file.static_ui.file_name.is_empty() {
        None
    } else {
        Some(state_file.static_ui.file_name.as_str())
    };
    let disc_ui = discover_pc_static_ui(game_dir, cached_ui);

    let mut out = Vec::with_capacity(4);

    // 1. VStringPool (dynamically discovered!)
    match disc_vs {
        Some(vs) => {
            let bak_name = format!("{}.original", vs.file_name);
            let bak = bak_dir.join(&bak_name);
            let has_backup = bak.is_file();
            let sha = sha256_file(&vs.full_path).unwrap_or_default();
            let state = classify_bundle_state(
                &vs.file_name,
                &sha,
                &bak,
                &state_file.vstring,
                VSTRING_ORIG_SHAS,
                VSTRING_KNOWN_EN_SHAS,
                VSTRING_KNOWN_ID_SHAS,
                true,
            );
            let disp_cnt = if state_file.vstring.file_name == vs.file_name && state_file.vstring.matched_count > 0 {
                state_file.vstring.matched_count
            } else {
                VSTRING_STRINGS
            };
            let lvl = if state.starts_with("patched_") { LogLevel::Ok } else { LogLevel::Info };
            let state_lbl = match state.as_str() {
                "patched_en" => "ENGLISH",
                "patched_id" => "INDONESIAN",
                "patched_custom" => "CUSTOM",
                "original" => "ORIGINAL JP",
                _ => "MODIFIED",
            };
            log(
                &format!(
                    "  [{}] VStringPool [{}] ({} strings, pool={}, backup={})",
                    state_lbl,
                    vs.file_name,
                    disp_cnt,
                    vs.entry_count,
                    if has_backup { "YES" } else { "NO" }
                ),
                lvl,
            );
            out.push(TargetStatus {
                id: "vstring".into(),
                name: "VStringPool".into(),
                subtitle: "Story, Quests, Skills, Kibo, Settings".into(),
                rel_path: vs.rel_path,
                backup_name: bak_name,
                string_count: disp_cnt,
                state,
                has_backup,
                sha256: sha,
            });
        }
        None => {
            log("  [MISSING] VStringPool (raw_base_package/BundleFiles/*.r)", LogLevel::Warn);
            out.push(TargetStatus {
                id: "vstring".into(),
                name: "VStringPool".into(),
                subtitle: "Story, Quests, Skills, Kibo, Settings".into(),
                rel_path: RAW_BUNDLE_REL_DIR.into(),
                backup_name: String::new(),
                string_count: VSTRING_STRINGS,
                state: "missing".into(),
                has_backup: false,
                sha256: String::new(),
            });
        }
    }

    // 2. Static UI KeyConfig (dynamically discovered!)
    match disc_ui {
        Some(ui) => {
            let bak_name = format!("{}.original", ui.file_name);
            let bak = bak_dir.join(&bak_name);
            let has_backup = bak.is_file();
            let sha = sha256_file(&ui.full_path).unwrap_or_default();
            let state = classify_bundle_state(
                &ui.file_name,
                &sha,
                &bak,
                &state_file.static_ui,
                STATIC_UI_ORIG_SHAS,
                STATIC_UI_KNOWN_EN_SHAS,
                STATIC_UI_KNOWN_ID_SHAS,
                true,
            );
            let lvl = if state.starts_with("patched_") { LogLevel::Ok } else { LogLevel::Info };
            let state_lbl = match state.as_str() {
                "patched_en" => "ENGLISH",
                "patched_id" => "INDONESIAN",
                "patched_custom" => "CUSTOM",
                "original" => "ORIGINAL JP",
                _ => "MODIFIED",
            };
            log(
                &format!(
                    "  [{}] Static UI KeyConfig [{}] ({} strings, sub={}B, backup={})",
                    state_lbl,
                    ui.file_name,
                    STATIC_UI_STRINGS,
                    ui.sub_size,
                    if has_backup { "YES" } else { "NO" }
                ),
                lvl,
            );
            out.push(TargetStatus {
                id: "static_ui".into(),
                name: "Static UI KeyConfig".into(),
                subtitle: "Prefab Buttons, Headers, Notices".into(),
                rel_path: ui.rel_path,
                backup_name: bak_name,
                string_count: STATIC_UI_STRINGS,
                state,
                has_backup,
                sha256: sha,
            });
        }
        None => {
            log("  [MISSING] Static UI KeyConfig (default_package/BundleFiles/*.r)", LogLevel::Warn);
            out.push(TargetStatus {
                id: "static_ui".into(),
                name: "Static UI KeyConfig".into(),
                subtitle: "Prefab Buttons, Headers, Notices".into(),
                rel_path: DEFAULT_BUNDLE_REL_DIR.into(),
                backup_name: String::new(),
                string_count: STATIC_UI_STRINGS,
                state: "missing".into(),
                has_backup: false,
                sha256: String::new(),
            });
        }
    }

    // 3 & 4. StaticPackage CSV & Resources Assets
    let fixed_specs: [(&str, &str, &str, &str, &str, usize, &str, &str, &str, &BundlePatchState); 2] = [
        (
            "static_pkg",
            "StaticPackage CSV",
            "Boot & Updater Messages",
            STATIC_PKG_REL,
            "6pq4a7hlp7zidu91dvvww.r.original",
            STATIC_PKG_STRINGS,
            STATIC_PKG_ORIG_SHA,
            STATIC_PKG_EN_SHA,
            STATIC_PKG_ID_SHA,
            &state_file.static_pkg,
        ),
        (
            "resources",
            "Resources Assets",
            "Fallback Startup & SDK",
            RESOURCES_REL,
            "resources.assets.original",
            RESOURCES_STRINGS,
            RESOURCES_ORIG_SHA,
            RESOURCES_EN_SHA,
            RESOURCES_ID_SHA,
            &state_file.resources,
        ),
    ];

    for (id, name, sub, rel, bak_name, count, orig_sha, en_sha, id_sha, st_entry) in fixed_specs {
        let full = game_dir.join(rel);
        let bak = bak_dir.join(bak_name);
        let has_backup = bak.is_file();
        let fname = full.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if !full.is_file() {
            log(&format!("  [MISSING] {}", rel), LogLevel::Warn);
            out.push(TargetStatus {
                id: id.into(),
                name: name.into(),
                subtitle: sub.into(),
                rel_path: rel.into(),
                backup_name: bak_name.into(),
                string_count: count,
                state: "missing".into(),
                has_backup,
                sha256: String::new(),
            });
            continue;
        }
        let sha = sha256_file(&full).unwrap_or_default();
        let state = classify_bundle_state(
            &fname,
            &sha,
            &bak,
            st_entry,
            &[orig_sha],
            &[en_sha],
            &[id_sha],
            false,
        );
        let lvl = if state.starts_with("patched_") { LogLevel::Ok } else { LogLevel::Info };
        let state_lbl = match state.as_str() {
            "patched_en" => "ENGLISH",
            "patched_id" => "INDONESIAN",
            "patched_custom" => "CUSTOM",
            "original" => "ORIGINAL JP",
            "modified" => "MODIFIED",
            _ => "MISSING",
        };
        log(
            &format!(
                "  [{}] {} ({} strings, backup={})",
                state_lbl,
                name,
                count,
                if has_backup { "YES" } else { "NO" }
            ),
            lvl,
        );
        out.push(TargetStatus {
            id: id.into(),
            name: name.into(),
            subtitle: sub.into(),
            rel_path: rel.into(),
            backup_name: bak_name.into(),
            string_count: count,
            state,
            has_backup,
            sha256: sha,
        });
    }
    out
}

fn ensure_backup_v2<F>(
    target: &Path,
    bak: &Path,
    file_name: &str,
    state_entry: &BundlePatchState,
    known_en: &[&str],
    known_id: &[&str],
    log: &mut F,
) -> Result<String, String>
where
    F: FnMut(&str, LogLevel),
{
    if bak.is_file() {
        return if state_entry.file_name == file_name && !state_entry.orig_sha.is_empty() {
            Ok(state_entry.orig_sha.clone())
        } else {
            sha256_file(bak)
        };
    }
    let cur_sha = sha256_file(target)?;
    let known_patched = sha_in_list(&cur_sha, known_en)
        || sha_in_list(&cur_sha, known_id)
        || (state_entry.file_name == file_name
            && !state_entry.patched_sha.is_empty()
            && cur_sha.eq_ignore_ascii_case(&state_entry.patched_sha));
    if known_patched {
        log(
            "  Target is already patched; skipping backup overwrite so original backup is never polluted.",
            LogLevel::Warn,
        );
        return Ok(String::new());
    }
    if let Some(parent) = bak.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Create backup dir failed: {}", e))?;
    }
    fs::copy(target, bak).map_err(|e| format!("Backup copy failed: {}", e))?;
    log(
        &format!(
            "  Saved verified original backup: {} (SHA256={}...)",
            bak.file_name().unwrap().to_string_lossy(),
            &cur_sha[..12.min(cur_sha.len())]
        ),
        LogLevel::Ok,
    );
    Ok(cur_sha)
}

fn check_no_backup_switch_guard_v2(
    target: &Path,
    bak: &Path,
    file_name: &str,
    lang: PatchLang,
    custom: Option<&CustomPack>,
    state_entry: &BundlePatchState,
    known_en: &[&str],
    known_id: &[&str],
) -> Result<bool, String> {
    if bak.is_file() {
        return Ok(false);
    }
    let cur = sha256_file(target)?;
    let st_match = state_entry.file_name == file_name
        && !state_entry.patched_sha.is_empty()
        && cur.eq_ignore_ascii_case(&state_entry.patched_sha);
    let is_en = sha_in_list(&cur, known_en) || (st_match && state_entry.lang == "patched_en");
    let is_id = sha_in_list(&cur, known_id) || (st_match && state_entry.lang == "patched_id");
    let is_custom = st_match && state_entry.lang == "patched_custom";
    if custom.is_some() {
        if is_en || is_id || is_custom {
            return Err(
                "Target is already patched and _ENPatchBackup is missing. Restore Original JP files before applying a Custom Language Pack."
                    .into(),
            );
        }
        return Ok(false);
    }
    if is_custom {
        return Err(
            "Target is currently patched with a Custom Language Pack and _ENPatchBackup is missing. Restore Original JP files first."
                .into(),
        );
    }
    match lang {
        PatchLang::En if is_en && st_match => Ok(true),
        PatchLang::Id if is_id && st_match => Ok(true),
        PatchLang::En if is_en || is_id => Err(
            "Target is already patched by an earlier version and _ENPatchBackup is missing. Restore Original JP files before updating/switching."
                .into(),
        ),
        PatchLang::Id if is_en || is_id => Err(
            "Target is already patched by an earlier version and _ENPatchBackup is missing. Restore Original JP files before updating/switching."
                .into(),
        ),
        _ => Ok(false),
    }
}

fn write_exclusive_verified(target: &Path, new_bytes: &[u8], backup_for_rollback: &Path) -> Result<(), String> {
    let prev_bytes = if backup_for_rollback.is_file() {
        fs::read(backup_for_rollback).map_err(|e| format!("Read rollback backup failed: {}", e))?
    } else {
        fs::read(target).map_err(|e| format!("Read current target failed: {}", e))?
    };

    // Exclusive open (share_mode = 0 => no concurrent readers/writers)
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(target)
        .map_err(|e| format!("Exclusive open {:?} failed (is the game open?): {}", target, e))?;

    let cur_len = f.metadata().map_err(|e| format!("Stat failed: {}", e))?.len() as usize;
    if cur_len != new_bytes.len() {
        return Err(format!(
            "Target size mismatch ({} != {})",
            cur_len,
            new_bytes.len()
        ));
    }

    let write_res = (|| -> Result<(), String> {
        f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        f.write_all(new_bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
        Ok(())
    })();

    if let Err(err) = write_res {
        let _ = f.seek(SeekFrom::Start(0));
        let _ = f.write_all(&prev_bytes);
        let _ = f.sync_all();
        return Err(format!("Write failed (auto-rolled back safely): {}", err));
    }
    Ok(())
}

fn load_rzv2_vstring(gz: &[u8]) -> Result<(HashMap<(u64, u32), Vec<u8>>, HashMap<usize, (u64, u32)>), String> {
    let raw = gunzip(gz)?;
    if raw.len() < 8 || &raw[0..4] != b"RZV2" {
        return Err("Corrupt RZV2 VStringPool payload header".into());
    }
    let count = read_u32_le(&raw, 4) as usize;
    let mut by_hash = HashMap::with_capacity(count);
    let mut pc_id_to_jp = HashMap::with_capacity(count);
    let mut p = 8usize;
    for _ in 0..count {
        if p + 20 > raw.len() {
            return Err("Truncated RZV2 record header".into());
        }
        let pc_id = read_u32_le(&raw, p) as usize;
        let jp_hash = u64::from_le_bytes([
            raw[p + 4], raw[p + 5], raw[p + 6], raw[p + 7],
            raw[p + 8], raw[p + 9], raw[p + 10], raw[p + 11],
        ]);
        let jp_len = read_u32_le(&raw, p + 12);
        let tr_len = read_u32_le(&raw, p + 16) as usize;
        p += 20;
        if p + tr_len > raw.len() {
            return Err("Truncated RZV2 record payload".into());
        }
        let key = (jp_hash, jp_len);
        by_hash.insert(key, raw[p..p + tr_len].to_vec());
        pc_id_to_jp.insert(pc_id, key);
        p += tr_len;
    }
    Ok((by_hash, pc_id_to_jp))
}

pub fn build_vstring_lookup(
    lang: PatchLang,
    custom: Option<&CustomPack>,
    allow_direct_ids: bool,
) -> Result<(HashMap<(u64, u32), Vec<u8>>, HashMap<usize, Vec<u8>>), String> {
    let (mut by_hash, pc_id_to_jp) = match custom {
        Some(c) => match c.base {
            CustomBase::En => load_rzv2_vstring(EMBED_VSTRING_EN_GZ)?,
            CustomBase::Id => load_rzv2_vstring(EMBED_VSTRING_ID_GZ)?,
            CustomBase::Jp => {
                let (_, id_map) = load_rzv2_vstring(EMBED_VSTRING_EN_GZ)?;
                (HashMap::with_capacity(c.vstring.len()), id_map)
            }
        },
        None => match lang {
            PatchLang::En => load_rzv2_vstring(EMBED_VSTRING_EN_GZ)?,
            PatchLang::Id => load_rzv2_vstring(EMBED_VSTRING_ID_GZ)?,
        },
    };
    let mut direct_overrides: HashMap<usize, Vec<u8>> = HashMap::new();
    if let Some(c) = custom {
        for (&pc_id, bytes) in &c.vstring {
            if let Some(&fp) = pc_id_to_jp.get(&pc_id) {
                by_hash.insert(fp, bytes.clone());
            } else if allow_direct_ids {
                direct_overrides.insert(pc_id, bytes.clone());
            }
        }
    }
    Ok((by_hash, direct_overrides))
}

fn load_string_dict(gz: &[u8]) -> Result<HashMap<String, String>, String> {
    let raw = gunzip(gz)?;
    let count = read_u32_le(&raw, 0) as usize;
    let mut map = HashMap::with_capacity(count);
    let mut p = 4usize;
    for _ in 0..count {
        let klen = (raw[p] as usize) | ((raw[p + 1] as usize) << 8);
        p += 2;
        let k = String::from_utf8_lossy(&raw[p..p + klen]).into_owned();
        p += klen;
        let vlen = read_u32_le(&raw, p) as usize;
        p += 4;
        let v = String::from_utf8_lossy(&raw[p..p + vlen]).into_owned();
        p += vlen;
        map.insert(k, v);
    }
    Ok(map)
}

fn ensure_no_running_game_processes(action: &str) -> Result<(), String> {
    let procs = find_running_game_processes();
    if !procs.is_empty() {
        let s = procs
            .iter()
            .map(|p| format!("{} (PID {})", p.name, p.pid))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!("Blocked: {} is currently running. Close it before {}!", s, action));
    }
    Ok(())
}

pub fn apply_patch<L, P>(
    game_dir: &Path,
    lang: PatchLang,
    custom: Option<&CustomPack>,
    log: L,
    on_progress: P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    ensure_no_running_game_processes("patching")?;
    apply_patch_inner(game_dir, lang, custom, log, on_progress)
}

pub fn apply_patch_for_selftest<L, P>(
    game_dir: &Path,
    lang: PatchLang,
    custom: Option<&CustomPack>,
    log: L,
    on_progress: P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    apply_patch_inner(game_dir, lang, custom, log, on_progress)
}

fn apply_patch_inner<L, P>(
    game_dir: &Path,
    lang: PatchLang,
    custom: Option<&CustomPack>,
    mut log: L,
    mut on_progress: P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    let bak_dir = game_dir.join("_ENPatchBackup");
    fs::create_dir_all(&bak_dir).map_err(|e| format!("Failed creating backup dir: {}", e))?;
    let mut state_file = load_patch_state(&bak_dir);

    let active_label = match custom {
        Some(c) => format!("{} ({} · base={})", c.meta.name, c.meta.code, c.base.as_str().to_uppercase()),
        None => lang.label().to_string(),
    };
    let active_code = match custom {
        Some(c) => c.meta.code.as_str(),
        None => lang.short_code(),
    };

    let mut rewritten = 0usize;
    log(
        &format!("=== Starting {} Patch Installation ===", active_label),
        LogLevel::Info,
    );
    if let Some(c) = custom {
        log(
            &format!(
                "  Custom Pack: {} ({} vstring + {} static_ui + {} builtin overrides)",
                c.meta.file_name, c.meta.vstring_count, c.meta.static_ui_count, c.meta.builtin_count
            ),
            LogLevel::Info,
        );
    }
    on_progress(0, TOTAL_STRINGS, &format!("Unpacking {} payloads...", active_code));

    // [1/4] VStringPool (universal hash-keyed, up to 76,180 strings)
    log(
        &format!("=== [1/4] Patching VStringPool ({}) ===", active_code),
        LogLevel::Info,
    );
    rewritten += patch_vstring_pool(
        game_dir,
        &bak_dir,
        &mut state_file,
        lang,
        custom,
        &mut log,
        &mut |delta, stage| on_progress(rewritten + delta, TOTAL_STRINGS, stage),
    )?;
    save_patch_state(&bak_dir, &state_file);

    // [2/4] Static UI KeyConfig (3,135 strings, dynamically discovered!)
    log(
        &format!("=== [2/4] Patching Static UI KeyConfig ({}) ===", active_code),
        LogLevel::Info,
    );
    rewritten += patch_static_ui(
        game_dir,
        &bak_dir,
        &mut state_file,
        lang,
        custom,
        &mut log,
        &mut |delta, stage| on_progress(rewritten + delta, TOTAL_STRINGS, stage),
    )?;
    save_patch_state(&bak_dir, &state_file);

    // [3/4] StaticPackage updatableBuiltInLangString (113 strings)
    log(
        &format!("=== [3/4] Patching Startup/Updater StaticPackage ({}) ===", active_code),
        LogLevel::Info,
    );
    rewritten += patch_static_pkg(
        game_dir,
        &bak_dir,
        &mut state_file,
        lang,
        custom,
        &mut log,
        &mut |delta, stage| on_progress(rewritten + delta, TOTAL_STRINGS, stage),
    )?;
    save_patch_state(&bak_dir, &state_file);

    // [4/4] resources.assets (358 strings)
    log(
        &format!("=== [4/4] Patching Startup/SDK Resources ({}) ===", active_code),
        LogLevel::Info,
    );
    rewritten += patch_resources_assets(
        game_dir,
        &bak_dir,
        &mut state_file,
        lang,
        custom,
        &mut log,
        &mut |delta, stage| on_progress(rewritten + delta, TOTAL_STRINGS, stage),
    )?;
    save_patch_state(&bak_dir, &state_file);

    on_progress(
        rewritten,
        rewritten.max(TOTAL_STRINGS),
        &format!("All 4 targets verified 100% ({})!", active_code),
    );
    log(
        &format!(
            "=== COMPLETE [{}]! Rewritten & verified: {} strings ===",
            active_label,
            rewritten
        ),
        LogLevel::Ok,
    );
    Ok(rewritten)
}

pub fn patch_vstring_universal_bytes<L, P>(
    orig_buf: &[u8],
    lang: PatchLang,
    custom: Option<&CustomPack>,
    allow_direct_ids: bool,
    log: &mut L,
    on_sub: &mut P,
) -> Result<(Vec<u8>, usize), String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, &str),
{
    if orig_buf.len() < 4144 {
        return Err("VStringPool file too small".into());
    }
    let code = custom.map(|c| c.meta.code.as_str()).unwrap_or_else(|| lang.short_code());
    on_sub(0, &format!("Loading universal {} dictionary (RZV2)...", code));
    let (by_hash, direct_overrides) = build_vstring_lookup(lang, custom, allow_direct_ids)?;
    log(
        &format!(
            "  Loaded {} universal {} VStringPool hash entries.",
            by_hash.len() + direct_overrides.len(),
            code
        ),
        LogLevel::Info,
    );

    on_sub(0, "Decrypting VStringPool (MMap)...");
    let mut h1 = orig_buf[4120..4144].to_vec();
    xor_mmap_in_place(&mut h1, 4120, EMBED_MMAP_KEY);
    let hash_bytes = read_u32_le(&h1, 0) as usize;
    let hash_start = read_u32_le(&h1, 4) as usize;
    if hash_start != 4120 {
        return Err(format!("Unexpected VStringHashWR startPos: {}", hash_start));
    }

    let str_sec_off = 4120 + hash_bytes;
    if str_sec_off + 20 > orig_buf.len() {
        return Err("VStringPool string section offset out of bounds".into());
    }
    let mut str_buf = orig_buf[str_sec_off..].to_vec();
    xor_mmap_in_place(&mut str_buf, str_sec_off, EMBED_MMAP_KEY);
    let count = read_u32_le(&str_buf, 16) as usize;
    if 20 + count.saturating_mul(4) > str_buf.len() {
        return Err(format!("VStringPool offset table out of bounds (count={})", count));
    }
    log(&format!("  Decrypted VStringPool: {} total strings.", count), LogLevel::Info);

    // Universal content-hash scan across all pool strings (hotfix-proof & platform-agnostic!)
    on_sub(0, &format!("Matching {} strings by content hash...", code));
    let mut matched_ids: Vec<usize> = Vec::with_capacity(by_hash.len().min(count));
    let mut matched_map: HashMap<usize, Vec<u8>> = HashMap::with_capacity(by_hash.len().min(count));
    for id in 0..count {
        let o_abs = read_u32_le(&str_buf, 20 + id * 4) as usize;
        if o_abs < str_sec_off {
            return Err(format!("VStringPool offset underflow at #{}", id));
        }
        let o_rel = o_abs - str_sec_off;
        if o_rel + 4 > str_buf.len() {
            return Err(format!("VStringPool length header OOB at #{}", id));
        }
        let o_len_i = read_i32_le(&str_buf, o_rel);
        if o_len_i < 0 {
            continue;
        }
        let o_len = o_len_i as usize;
        if o_rel + 4 + o_len > str_buf.len() {
            return Err(format!("VStringPool string payload OOB at #{}", id));
        }
        if let Some(tr) = direct_overrides.get(&id) {
            matched_ids.push(id);
            matched_map.insert(id, tr.clone());
            continue;
        }
        let raw_bytes = &str_buf[o_rel + 4..o_rel + 4 + o_len];
        let h = fnv1a64(raw_bytes);
        if let Some(tr) = by_hash.get(&(h, o_len as u32)) {
            matched_ids.push(id);
            matched_map.insert(id, tr.clone());
        }
    }
    if matched_ids.is_empty() {
        return Err("No matching VStringPool entries found in this bundle.".into());
    }
    log(
        &format!("  Matched {} strings dynamically across pool (count={}).", matched_ids.len(), count),
        LogLevel::Info,
    );

    // Group matched IDs into contiguous repackable spans (MAX_GAP=4096 bridges non-JP gaps cleanly)
    const MAX_GAP: usize = 4096;
    const MIN_SPAN_LEN: usize = 2;
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut inplace_ids: Vec<usize> = Vec::new();
    let mut s = matched_ids[0];
    let mut e = matched_ids[0];
    for &id in &matched_ids[1..] {
        if id - e <= MAX_GAP {
            e = id;
        } else {
            if e - s + 1 >= MIN_SPAN_LEN {
                spans.push((s, e));
            } else {
                inplace_ids.push(s);
            }
            s = id;
            e = id;
        }
    }
    if e - s + 1 >= MIN_SPAN_LEN {
        spans.push((s, e));
    } else {
        inplace_ids.push(s);
    }

    let mut new_str_buf = str_buf.clone();
    let total_tr = matched_ids.len();
    let mut rewritten = 0usize;

    for (span_idx, (start, end)) in spans.iter().copied().enumerate() {
        let span_start_abs = read_u32_le(&str_buf, 20 + start * 4) as usize;
        let span_end_abs = if end + 1 < count {
            read_u32_le(&str_buf, 20 + (end + 1) * 4) as usize
        } else {
            orig_buf.len()
        };
        if span_end_abs < span_start_abs {
            return Err(format!("Non-monotonic span offsets [{}..{}]", start, end));
        }
        let capacity = span_end_abs - span_start_abs;
        let mut packed = Vec::with_capacity(capacity);
        let mut cursor = span_start_abs;

        for id in start..=end {
            let payload: &[u8] = if let Some(tr) = matched_map.get(&id) {
                rewritten += 1;
                if rewritten % 2500 == 0 {
                    on_sub(
                        rewritten,
                        &format!(
                            "Rewriting VStringPool [{} / span {}]: {} / {}",
                            code,
                            span_idx + 1,
                            rewritten,
                            total_tr
                        ),
                    );
                }
                tr.as_slice()
            } else {
                let o_abs = read_u32_le(&str_buf, 20 + id * 4) as usize;
                let o_rel = o_abs - str_sec_off;
                let o_len = read_i32_le(&str_buf, o_rel) as usize;
                &str_buf[o_rel + 4..o_rel + 4 + o_len]
            };
            write_u32_le(&mut new_str_buf, 20 + id * 4, cursor as u32);
            packed.extend_from_slice(&(payload.len() as i32).to_le_bytes());
            packed.extend_from_slice(payload);
            cursor += 4 + payload.len();
        }
        let used = cursor - span_start_abs;
        if used > capacity {
            return Err(format!(
                "Span #{} [{}..{}] overflowed by {} bytes",
                span_idx + 1,
                start,
                end,
                used - capacity
            ));
        }
        let dst_off = span_start_abs - str_sec_off;
        new_str_buf[dst_off..dst_off + used].copy_from_slice(&packed);
        if used < capacity {
            new_str_buf[dst_off + used..dst_off + capacity].fill(0);
        }
        log(
            &format!(
                "  Repacked span #{} [#{}..#{}]: {}/{} B (free slack {} B)",
                span_idx + 1,
                start,
                end,
                used,
                capacity,
                capacity - used
            ),
            LogLevel::Diff,
        );
    }

    let mut skipped_inplace = 0usize;
    for id in inplace_ids {
        let o_abs = read_u32_le(&str_buf, 20 + id * 4) as usize;
        let o_rel = o_abs - str_sec_off;
        let o_len = read_i32_le(&str_buf, o_rel) as usize;
        let payload = &matched_map[&id];
        if payload.len() > o_len {
            // Gracefully skip isolated singleton that doesn't fit in-place (future hotfix safety)
            matched_map.remove(&id);
            skipped_inplace += 1;
            continue;
        }
        write_i32_le(&mut new_str_buf, o_rel, payload.len() as i32);
        new_str_buf[o_rel + 4..o_rel + 4 + payload.len()].copy_from_slice(payload);
        if payload.len() < o_len {
            new_str_buf[o_rel + 4 + payload.len()..o_rel + 4 + o_len].fill(0);
        }
        rewritten += 1;
    }
    if skipped_inplace > 0 {
        log(
            &format!("  Skipped {} isolated singleton string(s) exceeding in-place byte length.", skipped_inplace),
            LogLevel::Warn,
        );
    }

    on_sub(rewritten, &format!("Re-encrypting & verifying {} VStringPool strings...", rewritten));
    let mut out_buf = orig_buf.to_vec();
    out_buf[str_sec_off..].copy_from_slice(&new_str_buf);
    xor_mmap_in_place(&mut out_buf[str_sec_off..], str_sec_off, EMBED_MMAP_KEY);

    // Round-trip verification
    let mut v_sec = out_buf[str_sec_off..].to_vec();
    xor_mmap_in_place(&mut v_sec, str_sec_off, EMBED_MMAP_KEY);
    for (&id, expected) in &matched_map {
        let a = read_u32_le(&v_sec, 20 + id * 4) as usize;
        let rel = a - str_sec_off;
        let l = read_i32_le(&v_sec, rel) as usize;
        if l != expected.len() || &v_sec[rel + 4..rel + 4 + l] != expected.as_slice() {
            return Err(format!("VStringPool round-trip mismatch on #{}", id));
        }
    }
    if out_buf.len() != orig_buf.len() {
        return Err(format!("VStringPool output size mismatch ({} != {})", out_buf.len(), orig_buf.len()));
    }
    Ok((out_buf, rewritten))
}

pub fn patch_android_vstring_bytes<L, P>(
    orig_buf: &[u8],
    lang: PatchLang,
    custom: Option<&CustomPack>,
    log: &mut L,
    on_sub: &mut P,
) -> Result<(Vec<u8>, usize), String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, &str),
{
    patch_vstring_universal_bytes(orig_buf, lang, custom, false, log, on_sub)
}

fn patch_vstring_pool<L, P>(
    game_dir: &Path,
    bak_dir: &Path,
    state_file: &mut PatchStateFile,
    lang: PatchLang,
    custom: Option<&CustomPack>,
    log: &mut L,
    on_sub: &mut P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, &str),
{
    let disc = discover_pc_vstring(game_dir)
        .ok_or_else(|| "VStringPool bundle not found in raw_base_package\\BundleFiles\\".to_string())?;
    let target = &disc.full_path;
    let bak = bak_dir.join(format!("{}.original", disc.file_name));

    let orig_sha = ensure_backup_v2(
        target,
        &bak,
        &disc.file_name,
        &state_file.vstring,
        VSTRING_KNOWN_EN_SHAS,
        VSTRING_KNOWN_ID_SHAS,
        log,
    )?;
    if check_no_backup_switch_guard_v2(
        target,
        &bak,
        &disc.file_name,
        lang,
        custom,
        &state_file.vstring,
        VSTRING_KNOWN_EN_SHAS,
        VSTRING_KNOWN_ID_SHAS,
    )? {
        let cnt = state_file.vstring.matched_count.max(VSTRING_STRINGS);
        log(
            &format!("  VStringPool [{}] already matches {} ({} strings).", disc.file_name, lang.short_code(), cnt),
            LogLevel::Ok,
        );
        on_sub(cnt, "VStringPool already up-to-date");
        return Ok(cnt);
    }

    let src_path = if bak.is_file() { &bak } else { target };
    let code = custom.map(|c| c.meta.code.as_str()).unwrap_or_else(|| lang.short_code());
    let orig_buf = fs::read(src_path).map_err(|e| format!("Read VStringPool failed: {}", e))?;
    let (out_buf, rewritten) = patch_vstring_universal_bytes(&orig_buf, lang, custom, true, log, on_sub)?;

    write_exclusive_verified(target, &out_buf, &bak)?;
    let post_sha = sha256_file(target)?;
    state_file.vstring = BundlePatchState {
        file_name: disc.file_name.clone(),
        sub_offset: 0,
        lang: state_tag_for(lang, custom),
        custom_code: custom.map(|c| c.meta.code.clone()).unwrap_or_default(),
        orig_sha: if orig_sha.is_empty() { state_file.vstring.orig_sha.clone() } else { orig_sha },
        patched_sha: post_sha.clone(),
        matched_count: rewritten,
    };
    log(
        &format!(
            "  VStringPool [{} · {}] installed & verified! Rewritten={}, SHA256={}",
            disc.file_name,
            code,
            rewritten,
            post_sha
        ),
        LogLevel::Ok,
    );
    on_sub(rewritten, &format!("VStringPool verified ({} strings)", rewritten));
    Ok(rewritten)
}

#[inline]
fn align4(n: usize) -> usize {
    (n + 3) & !3
}

fn find_obj0_size_offset(sf: &[u8], data_off: usize, orig_mb_size: usize) -> Option<usize> {
    // Fast path: Unity 2022 standard ObjectInfo byteSize offset
    if 3220 + 4 <= data_off && read_u32_le(sf, 3220) as usize == orig_mb_size {
        return Some(3220);
    }
    // Dynamic scan inside ObjectInfo table (< data_off): byteStart == 0 (8 bytes) followed by byteSize == orig_mb_size
    let target_u32 = orig_mb_size as u32;
    let mut o = 8usize;
    while o + 4 <= data_off {
        if read_u32_le(sf, o) == target_u32 && sf[o - 8..o] == [0u8; 8] {
            return Some(o);
        }
        o += 4;
    }
    // Broader fallback: any 4-byte-aligned u32 match near end of header
    let mut o2 = data_off.saturating_sub(512) & !3;
    while o2 + 4 <= data_off {
        if read_u32_le(sf, o2) == target_u32 {
            return Some(o2);
        }
        o2 += 4;
    }
    None
}

pub fn patch_static_ui_bytes<L, P>(
    orig_buf: &[u8],
    sub_offset: usize,
    lang: PatchLang,
    custom: Option<&CustomPack>,
    _log: &mut L,
    on_sub: &mut P,
) -> Result<(Vec<u8>, usize), String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, &str),
{
    let mut tr_map = match custom {
        Some(c) => match c.base {
            CustomBase::En => load_string_dict(EMBED_STATIC_UI_EN_GZ)?,
            CustomBase::Id => load_string_dict(EMBED_STATIC_UI_ID_GZ)?,
            CustomBase::Jp => HashMap::with_capacity(c.static_ui.len()),
        },
        None => match lang {
            PatchLang::En => load_string_dict(EMBED_STATIC_UI_EN_GZ)?,
            PatchLang::Id => load_string_dict(EMBED_STATIC_UI_ID_GZ)?,
        },
    };
    if let Some(c) = custom {
        for (k, v) in &c.static_ui {
            tr_map.insert(k.clone(), v.clone());
        }
    }
    let code = custom.map(|c| c.meta.code.as_str()).unwrap_or_else(|| lang.short_code());
    on_sub(0, "Decrypting UnityCN ast_staticTextKeyConfig.asset...");
    let bundle = UnityCnBundle::parse(orig_buf, sub_offset)?;
    let sub_size = bundle.fsize;
    if sub_offset + sub_size > orig_buf.len() {
        return Err(format!(
            "Static UI sub-bundle out of range ({} + {} > {})",
            sub_offset,
            sub_size,
            orig_buf.len()
        ));
    }
    if bundle.blocks.is_empty() {
        return Err("Static UI sub-bundle has 0 blocks".into());
    }
    let orig_sf = bundle.read_all()?;

    // Locate ast_staticTextKeyConfig dynamically inside the decrypted SerializedFile
    const MARKER: &[u8] = b"ast_staticTextKeyConfig";
    let marker_pos = find_bytes(&orig_sf, MARKER)
        .ok_or("ast_staticTextKeyConfig marker not found in Static UI bundle")?;
    if marker_pos < 32 || read_i32_le(&orig_sf, marker_pos - 4) as usize != MARKER.len() {
        return Err("Unexpected ast_staticTextKeyConfig MonoBehaviour layout".into());
    }
    let mb_name_off = marker_pos - 4;
    let data_off = mb_name_off - 28;
    let name_end = mb_name_off + 4 + align4(MARKER.len());

    let mut cur = name_end;
    let lang_len = read_i32_le(&orig_sf, cur) as usize;
    cur += 4;
    let lang_tag = String::from_utf8_lossy(&orig_sf[cur..cur + lang_len]).into_owned();
    cur = align4(cur + lang_len);
    let pair_count = read_i32_le(&orig_sf, cur) as usize;
    cur += 4;

    let mut new_mb = Vec::with_capacity(orig_sf.len().saturating_sub(data_off));
    new_mb.extend_from_slice(&orig_sf[data_off..name_end]);
    let write_str = |dst: &mut Vec<u8>, s: &str| {
        let b = s.as_bytes();
        dst.extend_from_slice(&(b.len() as i32).to_le_bytes());
        dst.extend_from_slice(b);
        let pad = align4(b.len()) - b.len();
        for _ in 0..pad {
            dst.push(0);
        }
    };
    write_str(&mut new_mb, &lang_tag);
    new_mb.extend_from_slice(&(pair_count as i32).to_le_bytes());

    let mut replaced = 0usize;
    for _ in 0..pair_count {
        let kl = read_i32_le(&orig_sf, cur) as usize;
        cur += 4;
        let k = String::from_utf8_lossy(&orig_sf[cur..cur + kl]).into_owned();
        cur = align4(cur + kl);
        let vl = read_i32_le(&orig_sf, cur) as usize;
        cur += 4;
        let orig_v = String::from_utf8_lossy(&orig_sf[cur..cur + vl]).into_owned();
        cur = align4(cur + vl);

        let v = if let Some(tr) = tr_map.get(&k) {
            replaced += 1;
            if replaced % 400 == 0 {
                on_sub(
                    replaced,
                    &format!("Rewriting Static UI KeyConfig [{}]: {} / {}", code, replaced, STATIC_UI_STRINGS),
                );
            }
            tr.as_str()
        } else {
            orig_v.as_str()
        };
        write_str(&mut new_mb, &k);
        write_str(&mut new_mb, v);
    }

    let orig_mb_size = cur - data_off;
    let obj0_size_off = find_obj0_size_offset(&orig_sf, data_off, orig_mb_size)
        .ok_or_else(|| format!("Could not locate MonoBehaviour ObjectInfo size={} in Static UI SF", orig_mb_size))?;

    if new_mb.len() > orig_mb_size {
        return Err(format!("Rebuilt Static UI exceeds slot ({} > {})", new_mb.len(), orig_mb_size));
    }

    let mut new_sf = orig_sf.clone();
    new_sf[data_off..data_off + orig_mb_size].fill(0);
    new_sf[data_off..data_off + new_mb.len()].copy_from_slice(&new_mb);
    write_u32_le(&mut new_sf, obj0_size_off, new_mb.len() as u32);

    on_sub(replaced, "Compressing (LZ4HC exact) & UnityCN encrypting Static UI...");
    let total_budget = sub_size - bundle.data_start;
    let mut comp_blocks: Vec<Vec<u8>> = Vec::with_capacity(bundle.blocks.len());
    let mut sf_pos = 0usize;
    let mut used_comp = 0usize;
    let last_idx = bundle.blocks.len() - 1;
    for (idx, blk) in bundle.blocks.iter().enumerate() {
        let slice = &new_sf[sf_pos..sf_pos + blk.u_size];
        sf_pos += blk.u_size;
        let mut c = if idx < last_idx {
            lz4_compress(slice, 1024)
        } else {
            if used_comp >= total_budget {
                return Err(format!("Static UI compressed blocks exceeded budget ({} >= {})", used_comp, total_budget));
            }
            lz4_compress_exact(slice, total_budget - used_comp, 1024)?
        };
        if (blk.flags & 0x100) != 0 {
            bundle.cipher.encrypt_block_in_place(&mut c, idx);
        }
        used_comp += c.len();
        comp_blocks.push(c);
    }

    let mut new_bi_plain = Vec::with_capacity(128);
    new_bi_plain.extend_from_slice(&bundle.data_hash);
    new_bi_plain.extend_from_slice(&(bundle.blocks.len() as u32).to_be_bytes());
    for (blk, c) in bundle.blocks.iter().zip(comp_blocks.iter()) {
        new_bi_plain.extend_from_slice(&(blk.u_size as u32).to_be_bytes());
        new_bi_plain.extend_from_slice(&(c.len() as u32).to_be_bytes());
        new_bi_plain.extend_from_slice(&blk.flags.to_be_bytes());
    }
    new_bi_plain.extend_from_slice(&(bundle.dirs.len() as u32).to_be_bytes());
    for d in &bundle.dirs {
        new_bi_plain.extend_from_slice(&d.offset.to_be_bytes());
        new_bi_plain.extend_from_slice(&d.size.to_be_bytes());
        new_bi_plain.extend_from_slice(&d.flags.to_be_bytes());
        new_bi_plain.extend_from_slice(d.name.as_bytes());
        new_bi_plain.push(0);
    }
    let new_bi_comp = lz4_compress(&new_bi_plain, 256);
    if bundle.blocks_info_off + new_bi_comp.len() > bundle.data_start {
        return Err("Compressed blocksInfo overflowed data_start".into());
    }

    let mut new_sub = vec![0u8; sub_size];
    new_sub[..bundle.header_prefix.len()].copy_from_slice(&bundle.header_prefix);
    write_i64_be(&mut new_sub, bundle.fsize_off, sub_size as i64);
    write_u32_be(&mut new_sub, bundle.cinfo_off, new_bi_comp.len() as u32);
    write_u32_be(&mut new_sub, bundle.cinfo_off + 4, new_bi_plain.len() as u32);
    new_sub[bundle.blocks_info_off..bundle.blocks_info_off + new_bi_comp.len()].copy_from_slice(&new_bi_comp);
    let mut wp = bundle.data_start;
    for c in &comp_blocks {
        new_sub[wp..wp + c.len()].copy_from_slice(c);
        wp += c.len();
    }

    let mut out_buf = orig_buf.to_vec();
    out_buf[sub_offset..sub_offset + sub_size].copy_from_slice(&new_sub);

    // Round-trip verify decrypted SerializedFile
    let v_bundle = UnityCnBundle::parse(&out_buf, sub_offset)?;
    let v_sf = v_bundle.read_all()?;
    if v_sf != new_sf {
        return Err("Static UI round-trip verification failed".into());
    }
    Ok((out_buf, replaced))
}

fn patch_static_ui<L, P>(
    game_dir: &Path,
    bak_dir: &Path,
    state_file: &mut PatchStateFile,
    lang: PatchLang,
    custom: Option<&CustomPack>,
    log: &mut L,
    on_sub: &mut P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, &str),
{
    let cached_ui = if state_file.static_ui.file_name.is_empty() {
        None
    } else {
        Some(state_file.static_ui.file_name.as_str())
    };
    let disc = discover_pc_static_ui(game_dir, cached_ui)
        .ok_or_else(|| "Static UI bundle not found in default_package\\BundleFiles\\".to_string())?;
    let target = &disc.full_path;
    let bak = bak_dir.join(format!("{}.original", disc.file_name));

    let orig_sha = ensure_backup_v2(
        target,
        &bak,
        &disc.file_name,
        &state_file.static_ui,
        STATIC_UI_KNOWN_EN_SHAS,
        STATIC_UI_KNOWN_ID_SHAS,
        log,
    )?;
    if check_no_backup_switch_guard_v2(
        target,
        &bak,
        &disc.file_name,
        lang,
        custom,
        &state_file.static_ui,
        STATIC_UI_KNOWN_EN_SHAS,
        STATIC_UI_KNOWN_ID_SHAS,
    )? {
        log(
            &format!("  Static UI KeyConfig [{}] already matches {} ({} strings).", disc.file_name, lang.short_code(), STATIC_UI_STRINGS),
            LogLevel::Ok,
        );
        on_sub(STATIC_UI_STRINGS, "Static UI already up-to-date");
        return Ok(STATIC_UI_STRINGS);
    }
    let src_path = if bak.is_file() { &bak } else { target };
    let code = custom.map(|c| c.meta.code.as_str()).unwrap_or_else(|| lang.short_code());
    let orig_buf = fs::read(src_path).map_err(|e| format!("Read Static UI failed: {}", e))?;
    let (out_buf, replaced) = patch_static_ui_bytes(&orig_buf, disc.sub_offset, lang, custom, log, on_sub)?;

    write_exclusive_verified(target, &out_buf, &bak)?;
    let post_sha = sha256_file(target)?;
    state_file.static_ui = BundlePatchState {
        file_name: disc.file_name.clone(),
        sub_offset: disc.sub_offset,
        lang: state_tag_for(lang, custom),
        custom_code: custom.map(|c| c.meta.code.clone()).unwrap_or_default(),
        orig_sha: if orig_sha.is_empty() { state_file.static_ui.orig_sha.clone() } else { orig_sha },
        patched_sha: post_sha.clone(),
        matched_count: replaced,
    };
    log(
        &format!(
            "  Static UI KeyConfig [{} · {}] installed & verified! Rewritten={}, SHA256={}",
            disc.file_name,
            code,
            replaced,
            post_sha
        ),
        LogLevel::Ok,
    );
    on_sub(replaced, &format!("Static UI KeyConfig verified ({} strings)", replaced));
    Ok(replaced)
}

fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_q = false;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if in_q {
            if ch == '"' {
                if i + 1 < chars.len() && chars[i + 1] == '"' {
                    field.push('"');
                    i += 1;
                } else {
                    in_q = false;
                }
            } else {
                field.push(ch);
            }
        } else if ch == '"' {
            in_q = true;
        } else if ch == ',' {
            row.push(std::mem::take(&mut field));
        } else if ch == '\n' {
            row.push(std::mem::take(&mut field));
            rows.push(std::mem::take(&mut row));
        } else if ch != '\r' {
            field.push(ch);
        }
        i += 1;
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn serialize_csv_field(s: &str) -> String {
    if s.contains('"') || s.contains(',') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| {
        let u = c as u32;
        (0x3040..=0x30FF).contains(&u) || (0x4E00..=0x9FFF).contains(&u)
    })
}

fn patch_static_pkg<L, P>(
    game_dir: &Path,
    bak_dir: &Path,
    state_file: &mut PatchStateFile,
    lang: PatchLang,
    custom: Option<&CustomPack>,
    log: &mut L,
    on_sub: &mut P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, &str),
{
    let target = game_dir.join(STATIC_PKG_REL);
    let fname = "6pq4a7hlp7zidu91dvvww.r";
    let bak = bak_dir.join("6pq4a7hlp7zidu91dvvww.r.original");
    if !target.is_file() {
        return Err(format!("StaticPackage bundle not found: {:?}", target));
    }
    let orig_sha = ensure_backup_v2(
        &target,
        &bak,
        fname,
        &state_file.static_pkg,
        &[STATIC_PKG_EN_SHA],
        &[STATIC_PKG_ID_SHA],
        log,
    )?;
    if check_no_backup_switch_guard_v2(
        &target,
        &bak,
        fname,
        lang,
        custom,
        &state_file.static_pkg,
        &[STATIC_PKG_EN_SHA],
        &[STATIC_PKG_ID_SHA],
    )? {
        log(
            &format!("  Startup StaticPackage already matches {} (113 strings).", lang.short_code()),
            LogLevel::Ok,
        );
        on_sub(113, "StaticPackage already up-to-date");
        return Ok(113);
    }
    let src_path = if bak.is_file() { &bak } else { &target };
    let code = custom.map(|c| c.meta.code.as_str()).unwrap_or_else(|| lang.short_code());

    let effective_base = match custom {
        Some(c) => c.base,
        None => match lang {
            PatchLang::En => CustomBase::En,
            PatchLang::Id => CustomBase::Id,
        },
    };
    let builtin_dict = match effective_base {
        CustomBase::En => load_string_dict(EMBED_BUILTIN_EN_GZ)?,
        CustomBase::Id => load_string_dict(EMBED_BUILTIN_ID_GZ)?,
        CustomBase::Jp => HashMap::new(),
    };
    let orig_buf = fs::read(src_path).map_err(|e| format!("Read StaticPackage failed: {}", e))?;
    on_sub(0, "Decrypting StaticPackage (updatableBuiltInLangString)...");
    let bundle = UnityCnBundle::parse(&orig_buf, 0)?;
    let tag = b"updatableBuiltInLangString";
    // Try Block #5 first, fallback to scanning all blocks if future client moves it
    let mut target_blk_idx = None;
    let mut blk5 = Vec::new();
    let mut tag_pos = 0usize;
    let order: Vec<usize> = std::iter::once(5)
        .filter(|&i| i < bundle.blocks.len())
        .chain((0..bundle.blocks.len()).filter(|&i| i != 5))
        .collect();
    for idx in order {
        if let Ok(b) = bundle.read_block(idx) {
            if let Some(pos) = find_bytes(&b, tag) {
                target_blk_idx = Some(idx);
                blk5 = b;
                tag_pos = pos;
                break;
            }
        }
    }
    let Some(blk_idx) = target_blk_idx else {
        log("  [WARN] updatableBuiltInLangString not found in StaticPackage; skipping gracefully.", LogLevel::Warn);
        return Ok(0);
    };

    let len_off = align4(tag_pos + tag.len());
    let csv_byte_len = read_i32_le(&blk5, len_off) as usize;
    let csv_data_off = len_off + 4;
    let has_bom = blk5[csv_data_off..csv_data_off + 3] == [0xEF, 0xBB, 0xBF];
    let csv_text = String::from_utf8_lossy(
        &blk5[csv_data_off + if has_bom { 3 } else { 0 }..csv_data_off + csv_byte_len],
    )
    .into_owned();

    let mut rows = parse_csv(&csv_text);
    let cht_idx = rows[0].iter().position(|x| x == "cht");
    let kr_idx = rows[0].iter().position(|x| x == "kr");
    let en_idx = rows[0].iter().position(|x| x == "en").ok_or("No 'en' column")?;
    let jp_idx = rows[0].iter().position(|x| x == "jp").ok_or("No 'jp' column")?;
    let mut replaced = 0usize;
    let clear_extra_cols = effective_base == CustomBase::Id
        || custom.map(|c| !c.builtin.is_empty()).unwrap_or(false);
    for r in rows.iter_mut().skip(1) {
        if r.len() <= jp_idx || r[0].is_empty() {
            continue;
        }
        if let Some(custom_val) = custom.and_then(|c| c.builtin.get(&r[0])) {
            if !custom_val.is_empty() {
                if clear_extra_cols {
                    r[en_idx] = custom_val.clone();
                    if let Some(ci) = cht_idx {
                        if ci < r.len() {
                            r[ci].clear();
                        }
                    }
                    if let Some(ki) = kr_idx {
                        if ki < r.len() {
                            r[ki].clear();
                        }
                    }
                }
                r[jp_idx] = custom_val.clone();
                replaced += 1;
                continue;
            }
        }
        let val = match effective_base {
            CustomBase::En => {
                let mut v = r[en_idx].trim().to_string();
                if v.is_empty() || has_cjk(&v) {
                    if let Some(fb) = builtin_dict.get(&r[0]) {
                        v = fb.clone();
                    }
                }
                v
            }
            CustomBase::Id => {
                if let Some(id_val) = builtin_dict.get(&r[0]) {
                    id_val.clone()
                } else {
                    r[en_idx].trim().to_string()
                }
            }
            CustomBase::Jp => String::new(),
        };
        if !val.is_empty() && !has_cjk(&val) {
            if clear_extra_cols {
                r[en_idx] = val.clone();
                if let Some(ci) = cht_idx {
                    if ci < r.len() {
                        r[ci].clear();
                    }
                }
                if let Some(ki) = kr_idx {
                    if ki < r.len() {
                        r[ki].clear();
                    }
                }
            }
            r[jp_idx] = val;
            replaced += 1;
        }
    }

    let mut out_csv_str = String::new();
    for r in &rows {
        for (j, f) in r.iter().enumerate() {
            if j > 0 {
                out_csv_str.push(',');
            }
            out_csv_str.push_str(&serialize_csv_field(f));
        }
        out_csv_str.push('\n');
    }
    let body = out_csv_str.as_bytes();
    let total_len = body.len() + if has_bom { 3 } else { 0 };
    if total_len > csv_byte_len {
        return Err(format!("Rebuilt CSV larger than slot ({} > {})", total_len, csv_byte_len));
    }
    let mut new_csv = vec![0x0Au8; csv_byte_len];
    let wp = if has_bom {
        new_csv[0..3].copy_from_slice(&[0xEF, 0xBB, 0xBF]);
        3
    } else {
        0
    };
    new_csv[wp..wp + body.len()].copy_from_slice(body);
    blk5[csv_data_off..csv_data_off + csv_byte_len].copy_from_slice(&new_csv);

    on_sub(replaced, &format!("Compressing (LZ4HC exact) & UnityCN encrypting Block #{}...", blk_idx));
    let blk5_info = &bundle.blocks[blk_idx];
    let mut comp5 = lz4_compress_exact(&blk5, blk5_info.c_size, 1024)?;
    if (blk5_info.flags & 0x100) != 0 {
        bundle.cipher.encrypt_block_in_place(&mut comp5, blk_idx);
    }

    let mut out_buf = orig_buf.clone();
    out_buf[blk5_info.file_offset..blk5_info.file_offset + comp5.len()].copy_from_slice(&comp5);

    // Round-trip verify Block
    let v_bundle = UnityCnBundle::parse(&out_buf, 0)?;
    let v_blk5 = v_bundle.read_block(blk_idx)?;
    if v_blk5 != blk5 {
        return Err("StaticPackage round-trip verification failed".into());
    }

    write_exclusive_verified(&target, &out_buf, &bak)?;
    let post_sha = sha256_file(&target)?;
    state_file.static_pkg = BundlePatchState {
        file_name: fname.to_string(),
        sub_offset: 0,
        lang: state_tag_for(lang, custom),
        custom_code: custom.map(|c| c.meta.code.clone()).unwrap_or_default(),
        orig_sha: if orig_sha.is_empty() { state_file.static_pkg.orig_sha.clone() } else { orig_sha },
        patched_sha: post_sha.clone(),
        matched_count: replaced,
    };
    log(
        &format!(
            "  Startup StaticPackage [{}] installed & verified! Rewritten={}, SHA256={}",
            code,
            replaced,
            post_sha
        ),
        LogLevel::Ok,
    );
    on_sub(replaced, &format!("StaticPackage verified ({} strings)", replaced));
    Ok(replaced)
}

fn patch_resources_assets<L, P>(
    game_dir: &Path,
    bak_dir: &Path,
    state_file: &mut PatchStateFile,
    lang: PatchLang,
    custom: Option<&CustomPack>,
    log: &mut L,
    on_sub: &mut P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, &str),
{
    let target = game_dir.join(RESOURCES_REL);
    let fname = "resources.assets";
    let bak = bak_dir.join("resources.assets.original");
    if !target.is_file() {
        return Err(format!("resources.assets not found: {:?}", target));
    }
    let orig_sha = ensure_backup_v2(
        &target,
        &bak,
        fname,
        &state_file.resources,
        &[RESOURCES_EN_SHA],
        &[RESOURCES_ID_SHA],
        log,
    )?;
    if check_no_backup_switch_guard_v2(
        &target,
        &bak,
        fname,
        lang,
        custom,
        &state_file.resources,
        &[RESOURCES_EN_SHA],
        &[RESOURCES_ID_SHA],
    )? {
        log(
            &format!("  resources.assets already matches {} (358 strings).", lang.short_code()),
            LogLevel::Ok,
        );
        on_sub(358, "resources.assets already up-to-date");
        return Ok(358);
    }

    let src_path = if bak.is_file() { &bak } else { &target };
    let effective_base = match custom {
        Some(c) => c.base,
        None => match lang {
            PatchLang::En => CustomBase::En,
            PatchLang::Id => CustomBase::Id,
        },
    };
    if effective_base == CustomBase::Jp {
        let orig_buf = fs::read(src_path).map_err(|e| format!("Read resources.assets failed: {}", e))?;
        write_exclusive_verified(&target, &orig_buf, &bak)?;
        state_file.resources = BundlePatchState {
            file_name: fname.to_string(),
            sub_offset: 0,
            lang: "original".into(),
            custom_code: String::new(),
            orig_sha: if orig_sha.is_empty() { state_file.resources.orig_sha.clone() } else { orig_sha },
            patched_sha: String::new(),
            matched_count: 0,
        };
        log(
            "  resources.assets kept as Original JP (custom pack base=jp).",
            LogLevel::Info,
        );
        on_sub(0, "resources.assets kept as Original JP");
        return Ok(0);
    }

    let (diff_gz, expected_sha, base_code) = match effective_base {
        CustomBase::En => (EMBED_RESOURCES_DIFF_EN_GZ, RESOURCES_EN_SHA, "EN"),
        CustomBase::Id => (EMBED_RESOURCES_DIFF_ID_GZ, RESOURCES_ID_SHA, "ID"),
        CustomBase::Jp => unreachable!(),
    };
    let diff = gunzip(diff_gz)?;
    let expected_size = read_u32_le(&diff, 0) as usize;
    let span_count = read_u32_le(&diff, 4) as usize;

    let mut out_buf = fs::read(src_path).map_err(|e| format!("Read resources.assets failed: {}", e))?;
    if out_buf.len() != expected_size {
        log(
            &format!(
                "  [WARN] resources.assets size changed ({} != {}) — safely skipped (core 99.5% localization intact).",
                out_buf.len(),
                expected_size
            ),
            LogLevel::Warn,
        );
        on_sub(0, "resources.assets skipped (binary changed)");
        return Ok(0);
    }
    let mut p = 8usize;
    for _ in 0..span_count {
        let off = read_u32_le(&diff, p) as usize;
        let len = read_u32_le(&diff, p + 4) as usize;
        p += 8;
        out_buf[off..off + len].copy_from_slice(&diff[p..p + len]);
        p += len;
    }

    let cand_sha = sha256_bytes(&out_buf);
    if !expected_sha.is_empty() && !cand_sha.eq_ignore_ascii_case(expected_sha) {
        log(
            &format!(
                "  [WARN] resources.assets layout differs (SHA256={}) — safely skipped.",
                &cand_sha[..12]
            ),
            LogLevel::Warn,
        );
        on_sub(0, "resources.assets skipped (layout changed)");
        return Ok(0);
    }
    write_exclusive_verified(&target, &out_buf, &bak)?;
    state_file.resources = BundlePatchState {
        file_name: fname.to_string(),
        sub_offset: 0,
        lang: state_tag_for(lang, custom),
        custom_code: custom.map(|c| c.meta.code.clone()).unwrap_or_default(),
        orig_sha: if orig_sha.is_empty() { state_file.resources.orig_sha.clone() } else { orig_sha },
        patched_sha: cand_sha.clone(),
        matched_count: RESOURCES_STRINGS,
    };
    log(
        &format!(
            "  resources.assets [{}] installed & verified! Rewritten=358, SHA256={}",
            base_code,
            cand_sha
        ),
        LogLevel::Ok,
    );
    on_sub(358, "resources.assets verified (358 strings)");
    Ok(358)
}

pub fn rollback_to_original<L, P>(
    game_dir: &Path,
    news_cache_dir: Option<&Path>,
    log: L,
    on_progress: P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    ensure_no_running_game_processes("rollback")?;
    rollback_to_original_inner(game_dir, news_cache_dir, log, on_progress)
}

pub fn rollback_to_original_for_selftest<L, P>(
    game_dir: &Path,
    news_cache_dir: Option<&Path>,
    log: L,
    on_progress: P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    rollback_to_original_inner(game_dir, news_cache_dir, log, on_progress)
}

fn rollback_to_original_inner<L, P>(
    game_dir: &Path,
    news_cache_dir: Option<&Path>,
    mut log: L,
    mut on_progress: P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    let bak_dir = game_dir.join("_ENPatchBackup");
    let targets = scan_game_folder(game_dir, |_, _| {});
    let total = targets.len();
    let mut state_file = load_patch_state(&bak_dir);
    let mut restored = 0usize;
    log("=== Starting Rollback to Original JP ===", LogLevel::Warn);

    for (i, t) in targets.iter().enumerate() {
        let full = game_dir.join(&t.rel_path);
        let bak = bak_dir.join(&t.backup_name);
        on_progress(i, total, &format!("Restoring {}...", t.name));

        if t.backup_name.is_empty() || !bak.is_file() {
            log(&format!("  [SKIP] No backup found for {} ({})", t.name, t.backup_name), LogLevel::Warn);
            continue;
        }
        if !full.is_file() {
            log(&format!("  [SKIP] Target missing: {}", t.rel_path), LogLevel::Warn);
            continue;
        }
        let bak_sha = sha256_file(&bak)?;
        let cur_sha = sha256_file(&full)?;
        if !cur_sha.eq_ignore_ascii_case(&bak_sha) {
            let bak_bytes = fs::read(&bak).map_err(|e| format!("Read backup failed: {}", e))?;
            write_exclusive_verified(&full, &bak_bytes, &full)?;
            let post_sha = sha256_file(&full)?;
            if !post_sha.eq_ignore_ascii_case(&bak_sha) {
                return Err(format!("Rollback SHA-256 verification failed for {}", t.name));
            }
            log(
                &format!("  [RESTORED] {} -> Original JP (SHA256={}...)", t.name, &post_sha[..16]),
                LogLevel::Ok,
            );
        } else {
            log(&format!("  [OK] {} is already Original JP.", t.name), LogLevel::Ok);
        }
        let st_entry = match t.id.as_str() {
            "vstring" => Some(&mut state_file.vstring),
            "static_ui" => Some(&mut state_file.static_ui),
            "static_pkg" => Some(&mut state_file.static_pkg),
            "resources" => Some(&mut state_file.resources),
            _ => None,
        };
        if let Some(st) = st_entry {
            st.lang = "original".into();
            st.custom_code.clear();
            st.patched_sha.clear();
            st.orig_sha = bak_sha;
        }
        restored += 1;
    }
    save_patch_state(&bak_dir, &state_file);

    if let Some(cache_dir) = news_cache_dir {
        match rollback_news_cache(cache_dir, &mut log) {
            Ok(n) if n > 0 => {
                log(
                    &format!("  [RESTORED] Cached News ({} announcements) -> Original JP", n),
                    LogLevel::Ok,
                );
            }
            Ok(_) => {}
            Err(e) => {
                log(&format!("  [WARN] News cache rollback skipped: {}", e), LogLevel::Warn);
            }
        }
    }

    on_progress(total, total, &format!("Rollback complete ({}/{} restored).", restored, total));
    log(&format!("=== Rollback complete! Restored {}/{} targets to Original JP ===", restored, total), LogLevel::Ok);
    Ok(restored)
}

// ---------- Cached In-Game News / Announcements (BestHTTP HTTPCache + Library v3) ----------

pub fn default_news_cache_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("USERPROFILE")
        .ok_or_else(|| "USERPROFILE environment variable not found".to_string())?;
    Ok(PathBuf::from(home)
        .join("AppData")
        .join("LocalLow")
        .join("YostarJP")
        .join("AzurPromiliaJP"))
}

struct NewsLibEntry {
    hash: Option<String>,
    hex_name: String,
    body_len_off: usize,
    body_len: usize,
}

fn read_7bit_str(buf: &[u8], p: &mut usize) -> Result<String, String> {
    let mut len = 0usize;
    let mut shift = 0u32;
    loop {
        if *p >= buf.len() {
            return Err("Unexpected EOF while reading 7-bit string length".into());
        }
        let b = buf[*p];
        *p += 1;
        len |= ((b & 0x7F) as usize) << shift;
        if (b & 0x80) == 0 {
            break;
        }
        shift += 7;
        if shift > 28 {
            return Err("Corrupt 7-bit string length in Library".into());
        }
    }
    if *p + len > buf.len() {
        return Err("Unexpected EOF while reading 7-bit string payload".into());
    }
    let s = String::from_utf8_lossy(&buf[*p..*p + len]).into_owned();
    *p += len;
    Ok(s)
}

fn extract_announcement_hash(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    let marker = "announcement/prod/";
    let idx = lower.find(marker)?;
    let rest = &lower[idx + marker.len()..];
    let suffix = "/index.html";
    let end = rest.find(suffix)?;
    let hash = &rest[..end];
    if hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(hash.to_string())
    } else {
        None
    }
}

fn parse_httpcache_library(buf: &[u8]) -> Result<Vec<NewsLibEntry>, String> {
    if buf.len() < 16 {
        return Err("Library file too small".into());
    }
    let ver = read_i32_le(buf, 0);
    if ver != 3 {
        return Err(format!("Unsupported HTTP cache Library version: {}", ver));
    }
    let cnt = read_i32_le(buf, 12) as usize;
    let mut p = 16usize;
    let mut out = Vec::with_capacity(cnt);
    for _ in 0..cnt {
        let url = read_7bit_str(buf, &mut p)?;
        if p + 8 + 4 + 17 + 8 > buf.len() {
            return Err("Library entry truncated before metadata".into());
        }
        p += 8; // LastAccess
        let body_len_off = p;
        let body_len = read_i32_le(buf, p) as usize;
        p += 4;
        p += 17; // ver3 reserved bytes
        let mapped_idx = u64::from_le_bytes([
            buf[p],
            buf[p + 1],
            buf[p + 2],
            buf[p + 3],
            buf[p + 4],
            buf[p + 5],
            buf[p + 6],
            buf[p + 7],
        ]);
        p += 8;
        let _etag = read_7bit_str(buf, &mut p)?;
        let _last_mod = read_7bit_str(buf, &mut p)?;
        if p + 41 > buf.len() {
            return Err("Library entry truncated in footer timestamps".into());
        }
        p += 41; // Expires(8) + Age(8) + MaxAge(8) + Date(8) + MustRevalidate(1) + Received(8)
        out.push(NewsLibEntry {
            hash: extract_announcement_hash(&url),
            hex_name: format!("{:X}", mapped_idx),
            body_len_off,
            body_len,
        });
    }
    if p != buf.len() {
        return Err(format!("Library parse length mismatch ({} != {})", p, buf.len()));
    }
    Ok(out)
}

fn find_http_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn update_http_content_length(hdr_bytes: &[u8], new_len: usize) -> Vec<u8> {
    let hdr_str = String::from_utf8_lossy(hdr_bytes);
    let is_gzip = hdr_str
        .lines()
        .any(|l| l.to_ascii_lowercase().starts_with("content-encoding:") && l.to_ascii_lowercase().contains("gzip"));
    if is_gzip {
        return hdr_bytes.to_vec();
    }
    let mut lines: Vec<String> = hdr_str.split("\r\n").map(|s| s.to_string()).collect();
    for line in &mut lines {
        if line.to_ascii_lowercase().starts_with("content-length:") {
            *line = format!("Content-Length: {}", new_len);
        }
    }
    lines.join("\r\n").into_bytes()
}

pub fn patch_news_cache<L, P>(
    cache_base: &Path,
    lang: PatchLang,
    log: L,
    on_progress: P,
) -> Result<(usize, usize), String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    ensure_no_running_game_processes("translating news")?;
    patch_news_cache_inner(cache_base, lang, log, on_progress)
}

pub fn patch_news_cache_for_selftest<L, P>(
    cache_base: &Path,
    lang: PatchLang,
    log: L,
    on_progress: P,
) -> Result<(usize, usize), String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    patch_news_cache_inner(cache_base, lang, log, on_progress)
}

fn patch_news_cache_inner<L, P>(
    cache_base: &Path,
    lang: PatchLang,
    mut log: L,
    mut on_progress: P,
) -> Result<(usize, usize), String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    let lib_path = cache_base.join("Library");
    let hc_dir = cache_base.join("HTTPCache");
    if !lib_path.is_file() || !hc_dir.is_dir() {
        return Err(
            "News cache not found in AppData\\LocalLow. Open the in-game News/Announcements screen once, close the client, then click Translate News again."
                .into(),
        );
    }

    let dict = match lang {
        PatchLang::En => load_string_dict(EMBED_NEWS_EN_GZ)?,
        PatchLang::Id => load_string_dict(EMBED_NEWS_ID_GZ)?,
    };

    log(
        &format!("=== Starting Cached News Translation ({}) ===", lang.label()),
        LogLevel::Info,
    );
    let live_lib = fs::read(&lib_path).map_err(|e| format!("Read Library failed: {}", e))?;
    let entries = parse_httpcache_library(&live_lib)?;

    let matched_total = entries
        .iter()
        .filter(|e| e.hash.as_ref().map(|h| dict.contains_key(h)).unwrap_or(false))
        .count();
    if matched_total == 0 {
        return Err(
            "No matching cached announcement HTML pages found yet. Open the in-game Announcements once, close the client, and try again."
                .into(),
        );
    }

    // Save verified original backups before first patch (never overwrite existing backup)
    let bak_dir = cache_base.join("_NewsPatchBackup");
    let bak_hc = bak_dir.join("HTTPCache");
    let bak_lib = bak_dir.join("Library");
    if !bak_lib.is_file() {
        let any_jp = entries.iter().any(|e| {
            if e.hash.is_none() {
                return false;
            }
            if let Ok(raw) = fs::read(hc_dir.join(&e.hex_name)) {
                if let Some(he) = find_http_header_end(&raw) {
                    return has_cjk(&String::from_utf8_lossy(&raw[he + 4..]));
                }
            }
            false
        });
        if any_jp {
            fs::create_dir_all(&bak_hc).map_err(|e| format!("Create news backup dir failed: {}", e))?;
            fs::copy(&lib_path, &bak_lib).map_err(|e| format!("Backup Library failed: {}", e))?;
            for e in &entries {
                if e.hash.is_some() {
                    let src = hc_dir.join(&e.hex_name);
                    let dst = bak_hc.join(&e.hex_name);
                    if src.is_file() && !dst.is_file() {
                        let _ = fs::copy(&src, &dst);
                    }
                }
            }
            log("  Saved verified original news cache backup in _NewsPatchBackup\\.", LogLevel::Ok);
        }
    }

    let mut new_lib = live_lib.clone();
    let mut patched = 0usize;
    for e in &entries {
        let Some(hash) = e.hash.as_ref() else { continue };
        let Some(html) = dict.get(hash) else { continue };

        let bak_file = bak_hc.join(&e.hex_name);
        let live_file = hc_dir.join(&e.hex_name);
        let src_file = if bak_file.is_file() { &bak_file } else { &live_file };
        if !src_file.is_file() {
            continue;
        }
        let raw = fs::read(src_file).map_err(|err| format!("Read HC/{} failed: {}", e.hex_name, err))?;
        let hdr_end = find_http_header_end(&raw)
            .ok_or_else(|| format!("Missing HTTP header terminator in HC/{}", e.hex_name))?;

        let new_body = html.as_bytes();
        let new_hdr = update_http_content_length(&raw[..hdr_end], new_body.len());
        let mut out_buf = Vec::with_capacity(new_hdr.len() + 4 + new_body.len());
        out_buf.extend_from_slice(&new_hdr);
        out_buf.extend_from_slice(b"\r\n\r\n");
        out_buf.extend_from_slice(new_body);

        fs::write(&live_file, &out_buf)
            .map_err(|err| format!("Write HC/{} failed: {}", e.hex_name, err))?;
        write_i32_le(&mut new_lib, e.body_len_off, new_body.len() as i32);
        patched += 1;
        log(
            &format!(
                "  [{}] HC/{} ({}): {} B -> {} B",
                lang.short_code(),
                e.hex_name,
                &hash[..8],
                e.body_len,
                new_body.len()
            ),
            LogLevel::Diff,
        );
        on_progress(
            patched,
            matched_total,
            &format!("Translating News [{}]: {} / {}", lang.short_code(), patched, matched_total),
        );
    }

    fs::write(&lib_path, &new_lib).map_err(|e| format!("Write Library failed: {}", e))?;

    // Round-trip verify updated Library + HTTPCache body lengths
    let v_lib = fs::read(&lib_path).map_err(|e| format!("Verify read Library failed: {}", e))?;
    let v_entries = parse_httpcache_library(&v_lib)?;
    for e in &v_entries {
        if e.hash.is_some() {
            let raw = fs::read(hc_dir.join(&e.hex_name))
                .map_err(|err| format!("Verify read HC/{} failed: {}", e.hex_name, err))?;
            let he = find_http_header_end(&raw).ok_or("Verify header end missing")?;
            let actual = raw.len() - (he + 4);
            if actual != e.body_len {
                return Err(format!(
                    "News cache verification mismatch on HC/{}: {} != {}",
                    e.hex_name, actual, e.body_len
                ));
            }
        }
    }

    on_progress(
        patched,
        matched_total,
        &format!("News translated & verified ({}/{} in {})!", patched, matched_total, lang.short_code()),
    );
    log(
        &format!(
            "=== COMPLETE [News {}]! Patched & verified: {}/{} cached announcements ===",
            lang.label(),
            patched,
            matched_total
        ),
        LogLevel::Ok,
    );
    Ok((patched, matched_total))
}

pub fn rollback_news_cache<L>(cache_base: &Path, log: &mut L) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
{
    let bak_dir = cache_base.join("_NewsPatchBackup");
    let bak_lib = bak_dir.join("Library");
    let bak_hc = bak_dir.join("HTTPCache");
    if !bak_lib.is_file() {
        return Ok(0);
    }
    let lib_path = cache_base.join("Library");
    let hc_dir = cache_base.join("HTTPCache");
    let orig_lib = fs::read(&bak_lib).map_err(|e| format!("Read news Library backup failed: {}", e))?;
    let entries = parse_httpcache_library(&orig_lib)?;
    let mut restored = 0usize;
    for e in &entries {
        let src = bak_hc.join(&e.hex_name);
        if src.is_file() {
            fs::copy(&src, hc_dir.join(&e.hex_name))
                .map_err(|err| format!("Restore HC/{} failed: {}", e.hex_name, err))?;
            restored += 1;
        }
    }
    fs::write(&lib_path, &orig_lib).map_err(|e| format!("Restore Library failed: {}", e))?;
    log(
        &format!("  Restored news Library + {} original HTML announcements.", restored),
        LogLevel::Info,
    );
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_universal_pc_r3219598_r3273853_and_android_r3219596() {
        let mut noop_log = |_: &str, _: LogLevel| {};
        let mut noop_prog = |_: usize, _: &str| {};

        // 1. PC r3219598 originals in ../backups/
        let pc_vs_old = Path::new("../backups/z5xzl3cg6ao0ilpbzykexg.r.original");
        let pc_ui_old = Path::new("../backups/yr49bxop0f3reujfsmyfca.r.original");
        if pc_vs_old.is_file() && pc_ui_old.is_file() {
            let vs_orig = fs::read(pc_vs_old).unwrap();
            let ui_orig = fs::read(pc_ui_old).unwrap();
            let (ui_off, ui_sz) = find_static_ui_subbundle_in_bytes(&ui_orig).expect("find r3219598 Static UI");
            assert_eq!(ui_off, 420062);
            assert_eq!(ui_sz, 83151);
            for lang in [PatchLang::En, PatchLang::Id] {
                let (vs_out, vs_cnt) = patch_vstring_universal_bytes(&vs_orig, lang, None, true, &mut noop_log, &mut noop_prog).unwrap();
                let (ui_out, ui_cnt) = patch_static_ui_bytes(&ui_orig, ui_off, lang, None, &mut noop_log, &mut noop_prog).unwrap();
                assert_eq!(vs_out.len(), vs_orig.len());
                assert_eq!(ui_out.len(), ui_orig.len());
                assert_eq!(vs_cnt, 74055);
                assert_eq!(ui_cnt, 3135);
                println!("PC_r3219598_{}_VSTRING={} (cnt={})", lang.short_code(), sha256_bytes(&vs_out), vs_cnt);
                println!("PC_r3219598_{}_STATIC_UI={} (cnt={})", lang.short_code(), sha256_bytes(&ui_out), ui_cnt);
            }
        }

        // 2. PC r3273853 hotfix originals in ../backups/r3273853/
        let pc_vs_new = Path::new("../backups/r3273853/heqmts9lcjdvbupu0x2wpg.r.original");
        let pc_ui_new = Path::new("../backups/r3273853/h3y7ujukwn9qbhoqhzkkg.r.original");
        if pc_vs_new.is_file() && pc_ui_new.is_file() {
            assert_eq!(probe_vstring_file(pc_vs_new), Some(1667303));
            let vs_orig = fs::read(pc_vs_new).unwrap();
            let ui_orig = fs::read(pc_ui_new).unwrap();
            let (ui_off, ui_sz) = find_static_ui_subbundle_in_bytes(&ui_orig).expect("find r3273853 Static UI");
            assert_eq!(ui_off, 420062);
            assert_eq!(ui_sz, 83081);
            for lang in [PatchLang::En, PatchLang::Id] {
                let (vs_out, vs_cnt) = patch_vstring_universal_bytes(&vs_orig, lang, None, true, &mut noop_log, &mut noop_prog).unwrap();
                let (ui_out, ui_cnt) = patch_static_ui_bytes(&ui_orig, ui_off, lang, None, &mut noop_log, &mut noop_prog).unwrap();
                assert_eq!(vs_out.len(), vs_orig.len());
                assert_eq!(ui_out.len(), ui_orig.len());
                assert_eq!(vs_cnt, 76180);
                assert_eq!(ui_cnt, 3135);
                println!("PC_r3273853_{}_VSTRING={} (cnt={})", lang.short_code(), sha256_bytes(&vs_out), vs_cnt);
                println!("PC_r3273853_{}_STATIC_UI={} (cnt={})", lang.short_code(), sha256_bytes(&ui_out), ui_cnt);
            }
        }

        // 3. Android r3219596 originals in %LOCALAPPDATA%\RuzaLang\AndroidBackups\emulator-5554\
        if let Ok(la) = std::env::var("LOCALAPPDATA") {
            let bak = PathBuf::from(la).join(r"RuzaLang\AndroidBackups\emulator-5554");
            let an_vs = bak.join("1qnamndresrohzjhuvt2lw.r.original");
            let an_ui = bak.join("nibdvaxcnfh3en5bnvhtw.r.original");
            if an_vs.is_file() && an_ui.is_file() {
                let vs_orig = fs::read(&an_vs).unwrap();
                let ui_orig = fs::read(&an_ui).unwrap();
                let (ui_off, ui_sz) = find_static_ui_subbundle_in_bytes(&ui_orig).expect("find Android Static UI");
                assert_eq!(ui_off, 4200075);
                assert_eq!(ui_sz, 83151);
                for lang in [PatchLang::En, PatchLang::Id] {
                    let (vs_out, vs_cnt) = patch_android_vstring_bytes(&vs_orig, lang, None, &mut noop_log, &mut noop_prog).unwrap();
                    let (ui_out, ui_cnt) = patch_static_ui_bytes(&ui_orig, ui_off, lang, None, &mut noop_log, &mut noop_prog).unwrap();
                    assert_eq!(vs_out.len(), vs_orig.len());
                    assert_eq!(ui_out.len(), ui_orig.len());
                    assert_eq!(vs_cnt, 74058);
                    assert_eq!(ui_cnt, 3135);
                    println!("ANDROID_r3219596_{}_VSTRING={} (cnt={})", lang.short_code(), sha256_bytes(&vs_out), vs_cnt);
                    println!("ANDROID_r3219596_{}_STATIC_UI={} (cnt={})", lang.short_code(), sha256_bytes(&ui_out), ui_cnt);
                }
            }
        }
    }

    #[test]
    fn verify_custom_pack_parsing_and_repack() {
        let mut noop_log = |_: &str, _: LogLevel| {};
        let mut noop_prog = |_: usize, _: &str| {};

        // 1. Structured custom pack template
        let pack = parse_custom_pack_str(
            CUSTOM_PACK_TEMPLATE_JSON,
            "template.json",
            "template.json",
        )
        .unwrap();
        assert_eq!(pack.meta.code, "CUSTOM");
        assert_eq!(pack.base, CustomBase::En);
        assert!(pack.vstring.contains_key(&1504449));
        assert!(pack.static_ui.contains_key("15DC305A"));
        assert!(pack.builtin.contains_key("confirm"));

        // 2. Raw flat dictionary drop-in (no meta/vstring wrapper)
        let raw_json = r#"{
            "_note": "ignored",
            "1504449": "Equipar",
            "15DC305A": "Equipar",
            "CB185163+1": "Confirmar",
            "confirm": "Confirmar"
        }"#;
        let raw_pack = parse_custom_pack_str(raw_json, "es-dropin.json", "es-dropin.json").unwrap();
        assert_eq!(raw_pack.meta.name, "es-dropin");
        assert_eq!(raw_pack.vstring.len(), 1);
        assert_eq!(raw_pack.static_ui.len(), 2);
        assert_eq!(raw_pack.builtin.len(), 1);

        // 3. Custom pack overlay on real Android bundles (if local backup exists)
        if let Ok(la) = std::env::var("LOCALAPPDATA") {
            let bak = PathBuf::from(la).join(r"RuzaLang\AndroidBackups\emulator-5554");
            let an_vs = bak.join("1qnamndresrohzjhuvt2lw.r.original");
            let an_ui = bak.join("nibdvaxcnfh3en5bnvhtw.r.original");
            if an_vs.is_file() && an_ui.is_file() {
                let es_json = r#"{
                    "meta": { "name": "Español Test", "code": "ES", "author": "Test", "base": "en" },
                    "vstring": { "1504449": "Equipar", "1526899": "Bloqueo Perfecto" },
                    "static_ui": { "15DC305A": "Equipar" }
                }"#;
                let es_pack = parse_custom_pack_str(es_json, "es.json", "es.json").unwrap();
                let vs_orig = fs::read(&an_vs).unwrap();
                let ui_orig = fs::read(&an_ui).unwrap();
                let (ui_off, _) = find_static_ui_subbundle_in_bytes(&ui_orig).unwrap();
                let (vs_en, _) = patch_android_vstring_bytes(&vs_orig, PatchLang::En, None, &mut noop_log, &mut noop_prog).unwrap();
                let (ui_en, _) = patch_static_ui_bytes(&ui_orig, ui_off, PatchLang::En, None, &mut noop_log, &mut noop_prog).unwrap();
                let (vs_out, vs_cnt) = patch_android_vstring_bytes(&vs_orig, PatchLang::En, Some(&es_pack), &mut noop_log, &mut noop_prog).unwrap();
                let (ui_out, ui_cnt) = patch_static_ui_bytes(&ui_orig, ui_off, PatchLang::En, Some(&es_pack), &mut noop_log, &mut noop_prog).unwrap();
                assert_eq!(vs_out.len(), vs_orig.len());
                assert_eq!(ui_out.len(), ui_orig.len());
                assert_eq!(vs_cnt, 74058);
                assert_eq!(ui_cnt, 3135);
                assert_ne!(sha256_bytes(&vs_out), sha256_bytes(&vs_en));
                assert_ne!(sha256_bytes(&ui_out), sha256_bytes(&ui_en));
            }
        }
    }
}
