#!/usr/bin/env node
/**
 * Ruza-Lang Custom Language Pack Validator (Zero Dependencies)
 *
 * Usage:
 *   node tools/validate-pack.mjs <path-to-pack.json>
 *
 * Checks:
 *   1. Valid UTF-8 JSON (structured { meta, vstring, static_ui, builtin } OR flat dictionary drop-in)
 *   2. Key format validation (numeric VStringPool IDs, 8-hex Static UI keys)
 *   3. Placeholder parity against canonical EN reference ({0}, {playerName}, %s, %d, | gender splits)
 *   4. Balanced Unity rich-text tags (<color=...>, <i>, <b>, <size=...>)
 *   5. Anti-wrap warnings for tight UI labels
 *   6. VStringPool UTF-8 byte budget estimate
 */

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, '..');

const args = process.argv.slice(2);
if (args.length === 0 || args[0] === '-h' || args[0] === '--help') {
  console.log('Usage: node tools/validate-pack.mjs <path-to-pack.json>');
  process.exit(args.length === 0 ? 1 : 0);
}

const packPath = path.resolve(args[0]);
if (!fs.existsSync(packPath)) {
  console.error(`[ERROR] File not found: ${packPath}`);
  process.exit(1);
}

let rawText;
try {
  rawText = fs.readFileSync(packPath, 'utf8').replace(/^\uFEFF/, '');
} catch (err) {
  console.error(`[ERROR] Could not read file: ${err.message}`);
  process.exit(1);
}

let root;
try {
  root = JSON.parse(rawText);
} catch (err) {
  console.error(`[ERROR] Invalid JSON syntax: ${err.message}`);
  process.exit(1);
}

if (!root || typeof root !== 'object' || Array.isArray(root)) {
  console.error('[ERROR] JSON root must be an object { ... }.');
  process.exit(1);
}

function isStaticUiKey(k) {
  return /^[0-9A-Fa-f]{8}(\+\d+)?$/.test(k);
}

const errors = [];
const warnings = [];

const meta = (root.meta && typeof root.meta === 'object' && !Array.isArray(root.meta)) ? root.meta : {};
const packName = (typeof meta.name === 'string' && meta.name.trim()) ? meta.name.trim() : path.basename(packPath, '.json');
const packCode = (typeof meta.code === 'string' && meta.code.trim())
  ? meta.code.trim().replace(/[^A-Za-z0-9_-]/g, '').slice(0, 8).toUpperCase() || 'CUSTOM'
  : 'CUSTOM';
const packBase = (typeof meta.base === 'string' ? meta.base.trim().toLowerCase() : 'en');
if (!['en', 'id', 'jp', 'none', 'orig', 'original'].includes(packBase)) {
  warnings.push(`Unknown meta.base "${meta.base}" — patcher defaults to "en". Use "en", "id", or "jp".`);
}

const vstring = {};
const staticUi = {};
const builtin = {};

const hasStructured =
  (root.vstring && typeof root.vstring === 'object' && !Array.isArray(root.vstring)) ||
  (root.static_ui && typeof root.static_ui === 'object' && !Array.isArray(root.static_ui)) ||
  (root.builtin && typeof root.builtin === 'object' && !Array.isArray(root.builtin));

if (hasStructured) {
  for (const [secName, target, kind] of [
    ['vstring', vstring, 'vstring'],
    ['static_ui', staticUi, 'static_ui'],
    ['builtin', builtin, 'builtin'],
  ]) {
    const sec = root[secName];
    if (sec === undefined) continue;
    if (!sec || typeof sec !== 'object' || Array.isArray(sec)) {
      errors.push(`"${secName}" must be a JSON object.`);
      continue;
    }
    for (const [k, v] of Object.entries(sec)) {
      if (k.startsWith('_')) continue;
      if (typeof v !== 'string') {
        errors.push(`${secName}["${k}"] must be a string (got ${typeof v}).`);
        continue;
      }
      if (kind === 'vstring' && !/^\d+$/.test(k)) {
        errors.push(`vstring key "${k}" must be a numeric PC VStringPool ID.`);
        continue;
      }
      if (kind === 'static_ui' && !isStaticUiKey(k)) {
        warnings.push(`static_ui key "${k}" does not match 8-hex KeyConfig format (e.g. "15DC305A" or "CB185163+1").`);
      }
      target[k] = v;
    }
  }
} else {
  for (const [k, v] of Object.entries(root)) {
    if (k === 'meta' || k.startsWith('_')) continue;
    if (typeof v !== 'string') continue;
    if (isStaticUiKey(k)) staticUi[k] = v;
    else if (/^\d+$/.test(k)) vstring[k] = v;
    else builtin[k] = v;
  }
}

const vsKeys = Object.keys(vstring);
const uiKeys = Object.keys(staticUi);
const biKeys = Object.keys(builtin);
const totalOverrides = vsKeys.length + uiKeys.length + biKeys.length;

if (totalOverrides === 0) {
  errors.push('Pack contains 0 entries. Add translations under "vstring", "static_ui", or "builtin" (or flat ID->text keys).');
}

// Load canonical EN references if available in patches/
const refVsPath = path.join(repoRoot, 'patches', 'full-en-3219598.json');
const refUiPath = path.join(repoRoot, 'patches', 'static-ui-en-3219598.json');
const refVs = fs.existsSync(refVsPath) ? JSON.parse(fs.readFileSync(refVsPath, 'utf8')) : null;
const refUi = fs.existsSync(refUiPath) ? JSON.parse(fs.readFileSync(refUiPath, 'utf8')) : null;

function extractTokens(str) {
  const braces = [...str.matchAll(/\{[^{}]+\}/g)].map((m) => m[0]).sort();
  const printf = [...str.matchAll(/%(?:\d+\$)?[sdif%]/g)].map((m) => m[0]).sort();
  const pipes = (str.match(/\|/g) || []).length;
  return { braces, printf, pipes };
}

function checkTagBalance(str) {
  const openTags = [];
  const tagRe = /<(\/?)(color|i|b|size)(?:=[^<>]+)?>/gi;
  let m;
  while ((m = tagRe.exec(str)) !== null) {
    const isClose = m[1] === '/';
    const name = m[2].toLowerCase();
    if (!isClose) {
      openTags.push(name);
    } else {
      const idx = openTags.lastIndexOf(name);
      if (idx === -1) return `Unmatched closing </${name}> tag`;
      openTags.splice(idx, 1);
    }
  }
  if (openTags.length > 0) return `Unclosed <${openTags[openTags.length - 1]}> tag`;
  return null;
}

function checkEntries(sectionName, entries, refMap) {
  let unknownIds = 0;
  let wrapWarnings = 0;
  for (const [k, val] of Object.entries(entries)) {
    const tagErr = checkTagBalance(val);
    if (tagErr) {
      errors.push(`${sectionName}["${k}"]: ${tagErr}`);
    }
    if (refMap) {
      const ref = refMap[k];
      if (ref === undefined) {
        unknownIds++;
        if (unknownIds <= 5) {
          warnings.push(`${sectionName}["${k}"]: ID not found in reference dictionary (may be skipped).`);
        }
        continue;
      }
      const rt = extractTokens(ref);
      const vt = extractTokens(val);
      if (JSON.stringify(rt.braces) !== JSON.stringify(vt.braces)) {
        errors.push(`${sectionName}["${k}"]: placeholder mismatch (expected ${JSON.stringify(rt.braces)}, got ${JSON.stringify(vt.braces)})`);
      }
      if (JSON.stringify(rt.printf) !== JSON.stringify(vt.printf)) {
        errors.push(`${sectionName}["${k}"]: format token mismatch (expected ${JSON.stringify(rt.printf)}, got ${JSON.stringify(vt.printf)})`);
      }
      if (rt.pipes !== vt.pipes) {
        warnings.push(`${sectionName}["${k}"]: '|' split count changed (ref=${rt.pipes}, pack=${vt.pipes}) — used for gender/choice variants.`);
      }
      // Flag tight UI buttons that grew excessively
      const plainRef = ref.replace(/<[^>]+>/g, '').trim();
      const plainVal = val.replace(/<[^>]+>/g, '').trim();
      if (plainRef.length > 0 && plainRef.length <= 12 && plainVal.length > Math.max(18, Math.ceil(plainRef.length * 2.4))) {
        wrapWarnings++;
        if (wrapWarnings <= 8) {
          warnings.push(`${sectionName}["${k}"]: tight UI label "${plainRef}" -> "${plainVal}" (${plainVal.length} chars) may wrap in-game.`);
        }
      }
    }
  }
  if (unknownIds > 5) warnings.push(`${sectionName}: ${unknownIds - 5} more unknown ID(s) omitted.`);
  if (wrapWarnings > 8) warnings.push(`${sectionName}: ${wrapWarnings - 8} more tight-label wrap warning(s) omitted.`);
}

checkEntries('vstring', vstring, refVs);
checkEntries('static_ui', staticUi, refUi);
checkEntries('builtin', builtin, null);

// Estimate VStringPool UTF-8 byte delta vs reference EN (if available)
let byteNote = '';
if (refVs && vsKeys.length > 0) {
  let deltaBytes = 0;
  for (const k of vsKeys) {
    if (refVs[k] !== undefined) {
      deltaBytes += Buffer.byteLength(vstring[k], 'utf8') - Buffer.byteLength(refVs[k], 'utf8');
    }
  }
  const sign = deltaBytes >= 0 ? '+' : '';
  byteNote = ` (VStringPool UTF-8 delta vs EN: ${sign}${deltaBytes.toLocaleString('en-US')} B)`;
  // Observed free slack in base_jp + hotfix_jp is ~2.2 MB on EN; warn if adding > 1.5 MB
  if (deltaBytes > 1_500_000) {
    warnings.push(`Large UTF-8 byte growth (+${deltaBytes.toLocaleString('en-US')} B vs EN) — verify exact-size repack fits without span overflow.`);
  }
}

console.log('=== Ruza-Lang Language Pack Validation ===');
console.log(`File      : ${path.basename(packPath)}`);
console.log(`Pack      : ${packName} (${packCode}) | base=${packBase.toUpperCase()}`);
console.log(`Overrides : ${totalOverrides.toLocaleString('en-US')} total (${vsKeys.length} vstring, ${uiKeys.length} static_ui, ${biKeys.length} builtin)${byteNote}`);
console.log(`Errors    : ${errors.length}`);
console.log(`Warnings  : ${warnings.length}`);

if (warnings.length > 0) {
  console.log('\n--- Warnings ---');
  for (const w of warnings.slice(0, 25)) console.log(`  [WARN] ${w}`);
  if (warnings.length > 25) console.log(`  ... and ${warnings.length - 25} more warning(s)`);
}

if (errors.length > 0) {
  console.log('\n--- Errors ---');
  for (const e of errors.slice(0, 30)) console.log(`  [ERR ] ${e}`);
  if (errors.length > 30) console.log(`  ... and ${errors.length - 30} more error(s)`);
  console.log('\n[FAILED] Fix the errors above before installing or submitting a PR.');
  process.exit(1);
}

console.log('\n[PASS] Pack is valid and ready to load in Ruza-Lang (PC & Android)!');
process.exit(0);
