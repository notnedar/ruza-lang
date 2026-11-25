import { fileURLToPath } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import crypto from 'node:crypto';
import { parseUnityCNBundle, lz4Compress, lz4CompressExact } from './unitycn-lib.mjs';
import { readStaticTextPairs, buildStaticTextSerializedFile } from './static-text-patcher.mjs';

const WORKSPACE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const RE_CJK = /[\u3040-\u309f\u30a0-\u30ff\u4e00-\u9fff]/;
const RE_TAG = /<\/?[a-zA-Z][a-zA-Z0-9_-]*(?:=[^<>]*)?>/g;
const RE_RUBY = /^<\/?ruby(?:=[^<>]*)?>$/i;
const RE_BRACE = /\{[^{}]+\}/g;
const RE_PRINTF = /%(?:\d+\$)?[+-]?(?:\d+)?(?:\.\d+)?[sdfuxXeEgG]/g;

function sha256(buf) { return crypto.createHash('sha256').update(buf).digest('hex').toUpperCase(); }
function strictTags(s) { return (s.match(RE_TAG) || []).filter(t => !RE_RUBY.test(t)); }
function strictBraces(s) { return (s.match(RE_BRACE) || []).filter(b => !RE_CJK.test(b)); }
function countMap(arr) { const m = new Map(); for (const x of arr || []) m.set(x, (m.get(x) || 0) + 1); return m; }
function mapsEqual(a, b) { if (a.size !== b.size) return false; for (const [k, v] of a) if (b.get(k) !== v) return false; return true; }
function normalizeId(jp, val) {
  if (typeof val !== 'string') return '';
  if (jp.includes('\n') && !jp.includes('\\n') && val.includes('\\n')) val = val.replace(/\\n/g, '\n');
  else if (jp.includes('\\n') && !jp.includes('\n') && val.includes('\n')) val = val.replace(/\n/g, '\\n');
  if (jp.includes('\r\n') && !val.includes('\r\n') && val.includes('\n')) val = val.replace(/\n/g, '\r\n');
  return val.replace(/ーー/g, '—').replace(/ー/g, '—');
}
function validatePair(jp, val) {
  if (typeof val !== 'string' || !val.trim()) return ['empty'];
  const errs = [];
  if (RE_CJK.test(val)) errs.push('leftover_cjk');
  if ((jp.match(/\|/g) || []).length !== (val.match(/\|/g) || []).length) errs.push('pipe_mismatch');
  if (!mapsEqual(countMap(strictTags(jp)), countMap(strictTags(val)))) errs.push('tag_mismatch');
  if (!mapsEqual(countMap(strictBraces(jp)), countMap(strictBraces(val)))) errs.push('brace_mismatch');
  if (!mapsEqual(countMap(jp.match(RE_PRINTF)), countMap(val.match(RE_PRINTF)))) errs.push('printf_mismatch');
  return errs;
}

const mode = process.argv[2] || 'qa'; // 'qa' or 'pack'

// 1. Merge & QA VStringPool ID (523 chunks)
const vIdx = JSON.parse(fs.readFileSync(path.join(WORKSPACE, 'work/chunks_index.json'), 'utf8'));
const vMerged = { _comment: 'Ruza-Lang educational Indonesian VStringPool dictionary', '642215': 'Med' };
const vFailed = [];
let vValid = 0, vExp = 0, vJpB = 0, vIdB = 0;

for (const ch of vIdx.chunks) {
  const ins = JSON.parse(fs.readFileSync(path.join(WORKSPACE, 'work/chunks_in', ch.file), 'utf8'));
  const outP = path.join(WORKSPACE, 'work/chunks_out_id', ch.file);
  vExp += ins.length;
  if (!fs.existsSync(outP)) {
    for (const it of ins) vFailed.push({ kind: 'vstring', chunk: ch.chunkId, id: it.id, jp: it.jp, errs: ['missing_chunk'] });
    continue;
  }
  const om = JSON.parse(fs.readFileSync(outP, 'utf8'));
  for (const it of ins) {
    const raw = om[String(it.id)];
    if (raw === undefined) { vFailed.push({ kind: 'vstring', chunk: ch.chunkId, id: it.id, jp: it.jp, errs: ['missing_id'] }); continue; }
    const val = normalizeId(it.jp, raw);
    const errs = validatePair(it.jp, val);
    if (errs.length) vFailed.push({ kind: 'vstring', chunk: ch.chunkId, id: it.id, jp: it.jp, got: val, errs });
    else { vMerged[String(it.id)] = val; vValid++; vJpB += Buffer.byteLength(it.jp, 'utf8'); vIdB += Buffer.byteLength(val, 'utf8'); }
  }
}
const canaryIdPath = path.join(WORKSPACE, 'patches/canary-settings-id.json');
if (fs.existsSync(canaryIdPath)) {
  const c = JSON.parse(fs.readFileSync(canaryIdPath, 'utf8'));
  for (const [k, v] of Object.entries(c)) if (!k.startsWith('_')) vMerged[k] = v;
}
fs.writeFileSync(path.join(WORKSPACE, 'patches/full-id-3219598.json'), JSON.stringify(vMerged), 'utf8');

// 2. Merge & QA Static UI ID (20 chunks)
const sIdx = JSON.parse(fs.readFileSync(path.join(WORKSPACE, 'work/static_chunks_index.json'), 'utf8'));
const sMerged = { _comment: 'Full Bahasa Indonesia static UI patch for ast_staticTextKeyConfig.asset (3,135 strings)' };
const sFailed = [];
let sValid = 0, sExp = 0;
for (let c = 0; c < sIdx.totalChunks; c++) {
  const fn = `chunk_${String(c).padStart(4, '0')}.json`;
  const ins = JSON.parse(fs.readFileSync(path.join(WORKSPACE, 'work/static_chunks_in', fn), 'utf8'));
  const outP = path.join(WORKSPACE, 'work/static_chunks_out_id', fn);
  sExp += ins.length;
  if (!fs.existsSync(outP)) {
    for (const it of ins) sFailed.push({ kind: 'static_ui', chunk: c, id: it.id, key: it.key, jp: it.jp, errs: ['missing_chunk'] });
    continue;
  }
  const om = JSON.parse(fs.readFileSync(outP, 'utf8'));
  for (const it of ins) {
    const raw = om[String(it.id)];
    if (raw === undefined) { sFailed.push({ kind: 'static_ui', chunk: c, id: it.id, key: it.key, jp: it.jp, errs: ['missing_id'] }); continue; }
    const val = normalizeId(it.jp, raw);
    const errs = validatePair(it.jp, val);
    if (errs.length) sFailed.push({ kind: 'static_ui', chunk: c, id: it.id, key: it.key, jp: it.jp, got: val, errs });
    else { sMerged[it.key] = val; sValid++; }
  }
}
fs.writeFileSync(path.join(WORKSPACE, 'patches/static-ui-id-3219598.json'), JSON.stringify(sMerged), 'utf8');

const allFailed = [...vFailed, ...sFailed];
fs.writeFileSync(path.join(WORKSPACE, 'work/repair_needed_id.json'), JSON.stringify(allFailed, null, 2), 'utf8');

console.log('=== INDONESIAN QA SUMMARY ===', {
  vstring: `${vValid}/${vExp} (${((vValid/vExp)*100).toFixed(3)}%)`,
  vstringFailed: vFailed.length,
  vstringBytesSaved: vJpB - vIdB,
  staticUi: `${sValid}/${sExp} (${((sValid/sExp)*100).toFixed(3)}%)`,
  staticUiFailed: sFailed.length,
  totalFailed: allFailed.length,
});

if (mode !== 'pack') process.exit(0);
if (allFailed.length > 0) throw new Error(`Cannot pack while ${allFailed.length} items still failed QA!`);

// 3. Build resources.assets ID candidate & exact 4-span binary diff + pack all 4 _id.bin.gz payloads!
const resDir = path.join(WORKSPACE, 'work/exe_resources');
fs.mkdirSync(resDir, { recursive: true });

// 3a. Pack vstring_id.bin.gz
{
  const entries = Object.entries(vMerged).filter(([k]) => !k.startsWith('_')).map(([k, v]) => [Number(k), Buffer.from(v, 'utf8')]).sort((a, b) => a[0] - b[0]);
  const parts = [Buffer.alloc(4)]; parts[0].writeUInt32LE(entries.length, 0);
  for (const [id, b] of entries) { const h = Buffer.alloc(8); h.writeUInt32LE(id, 0); h.writeUInt32LE(b.length, 4); parts.push(h, b); }
  const gz = zlib.gzipSync(Buffer.concat(parts), { level: 9 });
  fs.writeFileSync(path.join(resDir, 'vstring_id.bin.gz'), gz);
  console.log('Packed vstring_id.bin.gz:', { count: entries.length, gzBytes: gz.length });
}

// 3b. Pack static_ui_id.bin.gz
{
  const entries = Object.entries(sMerged).filter(([k]) => !k.startsWith('_')).map(([k, v]) => [Buffer.from(k, 'utf8'), Buffer.from(v, 'utf8')]);
  const parts = [Buffer.alloc(4)]; parts[0].writeUInt32LE(entries.length, 0);
  for (const [kb, vb] of entries) {
    const h = Buffer.alloc(2); h.writeUInt16LE(kb.length, 0);
    const v = Buffer.alloc(4); v.writeUInt32LE(vb.length, 0);
    parts.push(h, kb, v, vb);
  }
  const gz = zlib.gzipSync(Buffer.concat(parts), { level: 9 });
  fs.writeFileSync(path.join(resDir, 'static_ui_id.bin.gz'), gz);
  console.log('Packed static_ui_id.bin.gz:', { count: entries.length, gzBytes: gz.length });
}

// 3c. Pack builtin_id.bin.gz (113 startup/updater rows)
const builtinIdMap = JSON.parse(fs.readFileSync(path.join(WORKSPACE, 'work/sdk_builtin_id/builtin_id.json'), 'utf8'));
{
  const entries = Object.entries(builtinIdMap).map(([k, v]) => [Buffer.from(k, 'utf8'), Buffer.from(v, 'utf8')]);
  const parts = [Buffer.alloc(4)]; parts[0].writeUInt32LE(entries.length, 0);
  for (const [kb, vb] of entries) {
    const h = Buffer.alloc(2); h.writeUInt16LE(kb.length, 0);
    const v = Buffer.alloc(4); v.writeUInt32LE(vb.length, 0);
    parts.push(h, kb, v, vb);
  }
  const gz = zlib.gzipSync(Buffer.concat(parts), { level: 9 });
  fs.writeFileSync(path.join(resDir, 'builtin_id.bin.gz'), gz);
  console.log('Packed builtin_id.bin.gz:', { count: entries.length, gzBytes: gz.length });
}

// 3d. Build resources.assets ID (in-place patch of builtInLangString @943448 + SDK `ja` @1016688) & pack resources_diff_id.bin.gz!
const sdkIdMap = JSON.parse(fs.readFileSync(path.join(WORKSPACE, 'work/sdk_builtin_id/sdk_id.json'), 'utf8'));
function parseCsv(text) {
  const rows = []; let row = [], field = '', inQ = false;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (inQ) { if (ch === '"') { if (text[i+1] === '"') { field += '"'; i++; } else inQ = false; } else field += ch; }
    else { if (ch === '"') inQ = true; else if (ch === ',') { row.push(field); field = ''; } else if (ch === '\n') { row.push(field); rows.push(row); row = []; field = ''; } else if (ch !== '\r') field += ch; }
  }
  if (field.length || row.length) { row.push(field); rows.push(row); }
  return rows;
}
function serCsv(s) { return /[",\r\n]/.test(s) ? '"' + s.replace(/"/g, '""') + '"' : s; }

const resOrig = fs.readFileSync(path.join(WORKSPACE, 'backups/resources.assets.original'));
const resId = Buffer.from(resOrig);

// Patch builtInLangString @ 943448 (orig data len = 15190, padded slot up to align4)
{
  const origCsvLen = resOrig.readUInt32LE(943448);
  const raw = resOrig.subarray(943452, 943452 + origCsvLen);
  const hasBom = raw[0] === 0xef && raw[1] === 0xbb && raw[2] === 0xbf;
  const rows = parseCsv((hasBom ? raw.subarray(3) : raw).toString('utf8'));
  const jpIdx = rows[0].indexOf('jp');
  let rep = 0;
  for (let i = 1; i < rows.length; i++) {
    const r = rows[i];
    if (r.length > jpIdx && builtinIdMap[r[0]]) { r[jpIdx] = builtinIdMap[r[0]]; rep++; }
  }
  const newStr = rows.map(r => r.map(serCsv).join(',')).join('\n') + '\n';
  const newBuf = Buffer.concat(hasBom ? [Buffer.from([0xef, 0xbb, 0xbf]), Buffer.from(newStr, 'utf8')] : [Buffer.from(newStr, 'utf8')]);
  if (newBuf.length > origCsvLen) throw new Error(`resources builtInLangString ID overflow (${newBuf.length} > ${origCsvLen})`);
  // Keep exact slot by updating byteLen & object byteSize (just like Codex's verified patch!) and zero-padding remainder
  resId.writeUInt32LE(newBuf.length, 943448);
  resId.fill(0, 943452, 943452 + ((origCsvLen + 3) & ~3));
  newBuf.copy(resId, 943452);
  // Object byteSize at 15816 = (20-byte name "builtInLangString" + pad + 4B len + aligned data) = 28 + align4(newBuf.length)
  // Check exact delta from orig!
  const origAligned = (origCsvLen + 3) & ~3;
  const newAligned = (newBuf.length + 3) & ~3;
  const headerOverhead = resOrig.readUInt32LE(15816) - origAligned;
  resId.writeUInt32LE(headerOverhead + newAligned, 15816);
  console.log('resources.assets builtInLangString ID:', { rep, origCsvLen, newLen: newBuf.length });
}

// Patch SDK `ja` @ 1016688 (orig data len = 23328)
{
  const origSdkLen = resOrig.readUInt32LE(1016688);
  const raw = resOrig.subarray(1016692, 1016692 + origSdkLen);
  const hasBom = raw[0] === 0xef && raw[1] === 0xbb && raw[2] === 0xbf;
  const lines = (hasBom ? raw.subarray(3) : raw).toString('utf8').split(/\r?\n/);
  let rep = 0;
  const outLines = lines.map(line => {
    if (!line.trim() || line.trim().startsWith('/')) return line;
    const eq = line.indexOf('=');
    if (eq <= 0) return line;
    const k = line.slice(0, eq).trim();
    if (sdkIdMap[k]) { rep++; return k + '=' + sdkIdMap[k]; }
    return line;
  });
  const newBuf = Buffer.concat(hasBom ? [Buffer.from([0xef, 0xbb, 0xbf]), Buffer.from(outLines.join('\r\n'), 'utf8')] : [Buffer.from(outLines.join('\r\n'), 'utf8')]);
  if (newBuf.length > origSdkLen) throw new Error(`resources SDK ja ID overflow (${newBuf.length} > ${origSdkLen})`);
  resId.writeUInt32LE(newBuf.length, 1016688);
  resId.fill(0, 1016692, 1016692 + ((origSdkLen + 3) & ~3));
  newBuf.copy(resId, 1016692);
  const origAligned = (origSdkLen + 3) & ~3;
  const newAligned = (newBuf.length + 3) & ~3;
  const headerOverhead = resOrig.readUInt32LE(15936) - origAligned;
  resId.writeUInt32LE(headerOverhead + newAligned, 15936);
  console.log('resources.assets SDK ja ID:', { rep, origSdkLen, newLen: newBuf.length });
}

const resIdSha = sha256(resId);
// Build contiguous binary diff spans for resources_diff_id.bin.gz
{
  const spans = [];
  let i = 0;
  while (i < resOrig.length) {
    if (resOrig[i] !== resId[i]) {
      let start = i, last = i;
      while (i < resOrig.length) {
        if (resOrig[i] !== resId[i]) last = i;
        else if (i - last > 64) break;
        i++;
      }
      spans.push({ start, len: last - start + 1 });
    } else i++;
  }
  const parts = [Buffer.alloc(8)];
  parts[0].writeUInt32LE(resOrig.length, 0);
  parts[0].writeUInt32LE(spans.length, 4);
  for (const sp of spans) {
    const sh = Buffer.alloc(8); sh.writeUInt32LE(sp.start, 0); sh.writeUInt32LE(sp.len, 4);
    parts.push(sh, resId.subarray(sp.start, sp.start + sp.len));
  }
  const gz = zlib.gzipSync(Buffer.concat(parts), { level: 9 });
  fs.writeFileSync(path.join(resDir, 'resources_diff_id.bin.gz'), gz);
  console.log('Packed resources_diff_id.bin.gz:', { spans: spans.length, gzBytes: gz.length, resIdSha });
}
