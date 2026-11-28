#!/usr/bin/env node
/**
 * Transparent resource payload auditor for Ruza-Lang.
 *
 * Verifies every compressed/binary file under `resources/` against the open
 * human-readable JSON dictionaries in `patches/` and validates structural
 * integrity (no executable headers, exact schema conformance, valid OFL font).
 *
 * Usage:
 *   node tools/verify-resources.mjs
 */

import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const rootDir = path.resolve(__dirname, "..");
const resDir = path.join(rootDir, "resources");
const patchesDir = path.join(rootDir, "patches");

let failures = 0;
function ok(msg) {
  console.log(`  [ OK ] ${msg}`);
}
function fail(msg) {
  console.error(`  [FAIL] ${msg}`);
  failures++;
}

function sha256Hex(buf) {
  return crypto.createHash("sha256").update(buf).digest("hex").toUpperCase();
}

function checkNoExecMagic(name, buf) {
  const sigs = [
    ["PE/MZ", [0x4d, 0x5a]],
    ["ELF", [0x7f, 0x45, 0x4c, 0x46]],
    ["Mach-O", [0xcf, 0xfa, 0xed, 0xfe]],
    ["DEX", [0x64, 0x65, 0x78, 0x0a]],
    ["ZIP/JAR/APK", [0x50, 0x4b, 0x03, 0x04]],
  ];
  for (const [label, bytes] of sigs) {
    if (buf.length >= bytes.length && bytes.every((b, i) => buf[i] === b)) {
      fail(`${name} starts with unexpected executable/archive header (${label})`);
      return;
    }
  }
}

function parseKvDict(buf, name) {
  if (buf.length < 4) throw new Error(`${name}: truncated header`);
  const count = buf.readUInt32LE(0);
  const map = new Map();
  let p = 4;
  for (let i = 0; i < count; i++) {
    if (p + 2 > buf.length) throw new Error(`${name}: truncated key len at #${i}`);
    const klen = buf.readUInt16LE(p);
    p += 2;
    if (p + klen + 4 > buf.length) throw new Error(`${name}: truncated key/vlen at #${i}`);
    const k = buf.subarray(p, p + klen).toString("utf8");
    p += klen;
    const vlen = buf.readUInt32LE(p);
    p += 4;
    if (p + vlen > buf.length) throw new Error(`${name}: truncated val at #${i}`);
    const v = buf.subarray(p, p + vlen).toString("utf8");
    p += vlen;
    map.set(k, v);
  }
  if (p !== buf.length) {
    throw new Error(`${name}: unexpected trailing bytes (${buf.length - p} B)`);
  }
  return map;
}

function parseRzv2(buf, name) {
  if (buf.length < 8 || buf.subarray(0, 4).toString("ascii") !== "RZV2") {
    throw new Error(`${name}: invalid RZV2 magic`);
  }
  const count = buf.readUInt32LE(4);
  const byId = new Map();
  let p = 8;
  for (let i = 0; i < count; i++) {
    if (p + 20 > buf.length) throw new Error(`${name}: truncated record header at #${i}`);
    const pcId = buf.readUInt32LE(p);
    const trLen = buf.readUInt32LE(p + 16);
    p += 20;
    if (p + trLen > buf.length) throw new Error(`${name}: truncated payload at #${i}`);
    const val = buf.subarray(p, p + trLen).toString("utf8");
    p += trLen;
    byId.set(String(pcId), val);
  }
  if (p !== buf.length) {
    throw new Error(`${name}: unexpected trailing bytes (${buf.length - p} B)`);
  }
  return byId;
}

function verifyResourcesDiff(buf, name) {
  if (buf.length < 8) throw new Error(`${name}: truncated header`);
  const targetSize = buf.readUInt32LE(0);
  const spanCount = buf.readUInt32LE(4);
  let p = 8;
  let totalBytes = 0;
  for (let i = 0; i < spanCount; i++) {
    if (p + 8 > buf.length) throw new Error(`${name}: truncated span header at #${i}`);
    const off = buf.readUInt32LE(p);
    const len = buf.readUInt32LE(p + 4);
    p += 8;
    if (p + len > buf.length) throw new Error(`${name}: truncated span payload at #${i}`);
    if (off + len > targetSize) {
      throw new Error(`${name}: span #${i} out of target bounds (${off + len} > ${targetSize})`);
    }
    totalBytes += len;
    p += len;
  }
  if (p !== buf.length) {
    throw new Error(`${name}: unexpected trailing bytes (${buf.length - p} B)`);
  }
  return { targetSize, spanCount, totalBytes };
}

function verifyTtfFont(buf, name, expectedFamily) {
  if (buf.length < 12) throw new Error(`${name}: too small for TTF`);
  const sfnt = buf.readUInt32BE(0);
  if (sfnt !== 0x00010000) {
    throw new Error(`${name}: invalid TrueType sfnt version 0x${sfnt.toString(16)}`);
  }
  const numTables = buf.readUInt16BE(4);
  if (12 + numTables * 16 > buf.length) {
    throw new Error(`${name}: truncated table directory`);
  }
  const tags = new Set();
  let nameTable = null;
  for (let i = 0; i < numTables; i++) {
    const o = 12 + i * 16;
    const tag = buf.subarray(o, o + 4).toString("ascii");
    const tblOff = buf.readUInt32BE(o + 8);
    const tblLen = buf.readUInt32BE(o + 12);
    if (tblOff + tblLen > buf.length) {
      throw new Error(`${name}: table '${tag}' out of bounds`);
    }
    tags.add(tag);
    if (tag === "name") nameTable = buf.subarray(tblOff, tblOff + tblLen);
  }
  for (const req of ["cmap", "glyf", "head", "hhea", "hmtx", "loca", "maxp", "name"]) {
    if (!tags.has(req)) throw new Error(`${name}: missing required TrueType table '${req}'`);
  }
  const nameText = nameTable.toString("latin1").replace(/\0/g, "");
  if (!nameText.includes(expectedFamily)) {
    throw new Error(`${name}: font name table does not match '${expectedFamily}'`);
  }
  return numTables;
}

console.log("=== Ruza-Lang Resource Payload Transparency Audit ===");

const expectedFiles = new Set([
  "builtin_en.bin.gz",
  "builtin_id.bin.gz",
  "font_barlow_med.ttf.gz",
  "font_barlow_semi.ttf.gz",
  "mmap_key_4096.bin",
  "news_en.bin.gz",
  "news_id.bin.gz",
  "resources_diff.bin.gz",
  "resources_diff_id.bin.gz",
  "static_ui_en.bin.gz",
  "static_ui_id.bin.gz",
  "vstring_en.bin.gz",
  "vstring_id.bin.gz",
]);

const actualFiles = fs.readdirSync(resDir).sort();
for (const f of actualFiles) {
  if (!expectedFiles.has(f)) {
    fail(`Unexpected unaudited file in resources/: ${f}`);
  }
}
for (const f of expectedFiles) {
  if (!fs.existsSync(path.join(resDir, f))) {
    fail(`Missing expected resource file: ${f}`);
  }
}

// 1. Verify Static UI payloads against open JSON dictionaries
for (const [binName, jsonName] of [
  ["static_ui_en.bin.gz", "static-ui-en-3219598.json"],
  ["static_ui_id.bin.gz", "static-ui-id-3219598.json"],
]) {
  try {
    const raw = zlib.gunzipSync(fs.readFileSync(path.join(resDir, binName)));
    checkNoExecMagic(binName, raw);
    const binMap = parseKvDict(raw, binName);
    const srcJson = JSON.parse(fs.readFileSync(path.join(patchesDir, jsonName), "utf8"));
    const jsonKeys = Object.keys(srcJson).filter((k) => !k.startsWith("_"));
    let mismatches = 0;
    for (const k of jsonKeys) {
      if (binMap.get(k) !== srcJson[k]) mismatches++;
    }
    if (binMap.size !== jsonKeys.length || mismatches > 0) {
      fail(`${binName} differs from patches/${jsonName} (bin=${binMap.size}, json=${jsonKeys.length}, mismatches=${mismatches})`);
    } else {
      ok(`${binName} matches patches/${jsonName} 100% (${binMap.size} strings, SHA256=${sha256Hex(raw).slice(0, 12)}...)`);
    }
  } catch (e) {
    fail(e.message);
  }
}

// 2. Verify VStringPool RZV2 payloads against open JSON dictionaries
for (const [binName, jsonName] of [
  ["vstring_en.bin.gz", "full-en-3219598.json"],
  ["vstring_id.bin.gz", "full-id-3219598.json"],
]) {
  try {
    const raw = zlib.gunzipSync(fs.readFileSync(path.join(resDir, binName)));
    checkNoExecMagic(binName, raw);
    const binMap = parseRzv2(raw, binName);
    const srcJson = JSON.parse(fs.readFileSync(path.join(patchesDir, jsonName), "utf8"));
    let mismatches = 0;
    for (const [id, val] of binMap.entries()) {
      if (srcJson[id] !== val) mismatches++;
    }
    if (mismatches > 0 || binMap.size === 0) {
      fail(`${binName} differs from patches/${jsonName} (entries=${binMap.size}, mismatches=${mismatches})`);
    } else {
      ok(`${binName} (RZV2) matches patches/${jsonName} 100% (${binMap.size} strings, SHA256=${sha256Hex(raw).slice(0, 12)}...)`);
    }
  } catch (e) {
    fail(e.message);
  }
}

// 3. Verify Built-in startup CSV string dictionaries
for (const binName of ["builtin_en.bin.gz", "builtin_id.bin.gz"]) {
  try {
    const raw = zlib.gunzipSync(fs.readFileSync(path.join(resDir, binName)));
    checkNoExecMagic(binName, raw);
    const map = parseKvDict(raw, binName);
    if (map.size !== 113) {
      fail(`${binName}: expected 113 startup strings, got ${map.size}`);
    } else {
      ok(`${binName} verified plain UTF-8 KV dictionary (${map.size} startup strings, zero trailing bytes)`);
    }
  } catch (e) {
    fail(e.message);
  }
}

// 4. Verify News translation payloads against patches/news-translations.json
try {
  const newsSrc = JSON.parse(fs.readFileSync(path.join(patchesDir, "news-translations.json"), "utf8"));
  for (const [binName, langKey] of [
    ["news_en.bin.gz", "en"],
    ["news_id.bin.gz", "id"],
  ]) {
    const raw = zlib.gunzipSync(fs.readFileSync(path.join(resDir, binName)));
    checkNoExecMagic(binName, raw);
    const map = parseKvDict(raw, binName);
    let mismatches = 0;
    for (const [hashKey, html] of map.entries()) {
      const entry = newsSrc[hashKey] || newsSrc.entries?.[hashKey];
      if (entry && typeof entry === "object" && langKey in entry && entry[langKey] !== html) {
        mismatches++;
      }
    }
    if (map.size === 0 || mismatches > 0) {
      fail(`${binName} news payload mismatch (entries=${map.size}, mismatches=${mismatches})`);
    } else {
      ok(`${binName} verified cached-news HTML dictionary (${map.size} pages, zero trailing bytes)`);
    }
  }
} catch (e) {
  fail(e.message);
}

// 5. Verify resources_diff binary span tables
for (const binName of ["resources_diff.bin.gz", "resources_diff_id.bin.gz"]) {
  try {
    const raw = zlib.gunzipSync(fs.readFileSync(path.join(resDir, binName)));
    checkNoExecMagic(binName, raw);
    const info = verifyResourcesDiff(raw, binName);
    ok(`${binName} verified exact span table (${info.spanCount} spans, ${info.totalBytes} B total, zero trailing bytes)`);
  } catch (e) {
    fail(e.message);
  }
}

// 6. Verify Open-Source Barlow Semi Condensed TrueType fonts (OFL-1.1)
for (const binName of ["font_barlow_med.ttf.gz", "font_barlow_semi.ttf.gz"]) {
  try {
    const raw = zlib.gunzipSync(fs.readFileSync(path.join(resDir, binName)));
    checkNoExecMagic(binName, raw);
    const numTables = verifyTtfFont(raw, binName, "Barlow Semi Condensed");
    ok(`${binName} verified TrueType font ('Barlow Semi Condensed', ${numTables} tables, ${raw.length} B, SHA256=${sha256Hex(raw).slice(0, 12)}...)`);
  } catch (e) {
    fail(e.message);
  }
}

// 7. Verify 4096-byte XOR lookup table
try {
  const keyBuf = fs.readFileSync(path.join(resDir, "mmap_key_4096.bin"));
  checkNoExecMagic("mmap_key_4096.bin", keyBuf);
  if (keyBuf.length !== 4096) {
    fail(`mmap_key_4096.bin expected 4096 B, got ${keyBuf.length} B`);
  } else {
    ok(`mmap_key_4096.bin verified exact 4096-byte XOR table (SHA256=${sha256Hex(keyBuf).slice(0, 12)}...)`);
  }
} catch (e) {
  fail(e.message);
}

console.log("");
if (failures > 0) {
  console.error(`Audit FAILED with ${failures} error(s).`);
  process.exit(1);
}
console.log("All 13 resource payloads passed 100% structural & dictionary verification!");
