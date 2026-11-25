import { fileURLToPath } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';

const WORKSPACE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const KEY_PATH = path.join(WORKSPACE, 'tools/mmap_key_4096.bin');

function sha256File(filePath) {
  const hash = crypto.createHash('sha256');
  const fd = fs.openSync(filePath, 'r');
  const buf = Buffer.alloc(4 * 1024 * 1024);
  let n;
  while ((n = fs.readSync(fd, buf, 0, buf.length, null)) > 0) {
    hash.update(buf.subarray(0, n));
  }
  fs.closeSync(fd);
  return hash.digest('hex').toUpperCase();
}

function xorMMapInPlace(buf, fileStartOffset, key4096) {
  for (let i = 0; i < buf.length; i++) {
    const pos = fileStartOffset + i;
    const shift = key4096[(pos >>> 12) & 0xFF];
    buf[i] ^= key4096[((pos & 0xFFF) + shift) & 0xFFF];
  }
  return buf;
}

function parseVStringMeta(fileBuf, key4096) {
  const h1 = Buffer.from(fileBuf.subarray(4120, 4144));
  xorMMapInPlace(h1, 4120, key4096);
  const hashBytes = h1.readUInt32LE(0);
  const hashStart = h1.readUInt32LE(4);
  if (hashStart !== 4120) {
    throw new Error(`Unexpected VStringHashWR startPos: ${hashStart}`);
  }
  const strSecOff = 4120 + hashBytes;
  const strBuf = Buffer.from(fileBuf.subarray(strSecOff));
  xorMMapInPlace(strBuf, strSecOff, key4096);

  const totalBytes = strBuf.readUInt32LE(0);
  const secStart = strBuf.readUInt32LE(4);
  const offTblStart = strBuf.readUInt32LE(8);
  const dataStart = strBuf.readUInt32LE(12);
  const count = strBuf.readUInt32LE(16);

  if (secStart !== strSecOff || offTblStart !== strSecOff + 20 || totalBytes !== strBuf.length) {
    throw new Error(`Unexpected VStringWR header: ${JSON.stringify({ totalBytes, secStart, offTblStart, dataStart, count, actualLen: strBuf.length })}`);
  }
  return { strSecOff, strBuf, totalBytes, dataStart, count };
}

function readStringAt(strBuf, strSecOff, idx) {
  const absOff = strBuf.readUInt32LE(20 + idx * 4);
  const rel = absOff - strSecOff;
  const slen = strBuf.readInt32LE(rel);
  if (slen < 0) return { absOff, slen, text: null };
  return { absOff, slen, text: strBuf.subarray(rel + 4, rel + 4 + slen).toString('utf8') };
}

function main() {
  const args = process.argv.slice(2);
  const getArg = (flag, def) => {
    const i = args.indexOf(flag);
    return i >= 0 && i + 1 < args.length ? args[i + 1] : def;
  };

  const originalPath = getArg('--original', path.join(WORKSPACE, 'backups/z5xzl3cg6ao0ilpbzykexg.r.original'));
  const patchPath = getArg('--patch', path.join(WORKSPACE, 'patches/canary-settings.json'));
  const outPath = getArg('--out', path.join(WORKSPACE, 'work/z5xzl3cg6ao0ilpbzykexg.r.english'));
  const targetPath = getArg('--target', './z5xzl3cg6ao0ilpbzykexg.r');
  const manifestOut = getArg('--manifest', path.join(WORKSPACE, 'work/vstring-patch-manifest.json'));

  const key4096 = fs.readFileSync(KEY_PATH);
  if (key4096.length !== 4096) throw new Error('Invalid mmap_key_4096.bin length');

  const patchObj = JSON.parse(fs.readFileSync(patchPath, 'utf8'));
  const translations = new Map();
  for (const [k, v] of Object.entries(patchObj)) {
    if (k.startsWith('_')) continue;
    const id = Number(k);
    if (!Number.isInteger(id) || typeof v !== 'string') {
      throw new Error(`Invalid translation entry: ${k}`);
    }
    translations.set(id, v);
  }
  console.log(`Loaded ${translations.size} translations from ${patchPath}`);

  const origBuf = fs.readFileSync(originalPath);
  const origSize = origBuf.length;
  const { strSecOff, strBuf, count } = parseVStringMeta(origBuf, key4096);
  console.log(`Parsed original VStringPool: size=${origSize}, count=${count}, strSecOff=${strSecOff}`);

  // Safe localization repack spans (only modify within these spans so all non-loc offsets stay untouched)
  const SPANS = [
    { name: 'base_jp', start: 1495181, end: 1563711 },
    { name: 'hotfix_jp', start: 1658279, end: count - 1 },
  ];

  // Validate all patch IDs fall inside safe spans (or fit in-place if outside)
  const spanIds = SPANS.map(() => []);
  const inplaceIds = [];
  for (const id of translations.keys()) {
    if (id <= 0 || id >= count) throw new Error(`String ID out of range: ${id}`);
    const sIdx = SPANS.findIndex(sp => id >= sp.start && id <= sp.end);
    if (sIdx >= 0) spanIds[sIdx].push(id);
    else inplaceIds.push(id);
  }

  // Clone strBuf for patching
  const newStrBuf = Buffer.from(strBuf);
  let replacedCount = 0;
  let totalOrigBytes = 0;
  let totalNewBytes = 0;

  // 1. Repack each active span that has at least 1 translation
  for (let sIdx = 0; sIdx < SPANS.length; sIdx++) {
    if (spanIds[sIdx].length === 0) continue;
    const sp = SPANS[sIdx];
    const spanStartAbs = strBuf.readUInt32LE(20 + sp.start * 4);
    const spanEndAbs = (sp.end + 1 < count)
      ? strBuf.readUInt32LE(20 + (sp.end + 1) * 4)
      : origSize;
    const capacity = spanEndAbs - spanStartAbs;

    const chunks = [];
    let cursor = spanStartAbs;
    for (let id = sp.start; id <= sp.end; id++) {
      const orig = readStringAt(strBuf, strSecOff, id);
      let payload;
      if (translations.has(id)) {
        payload = Buffer.from(translations.get(id), 'utf8');
        replacedCount++;
        totalOrigBytes += orig.slen;
        totalNewBytes += payload.length;
      } else {
        const rel = orig.absOff - strSecOff;
        payload = strBuf.subarray(rel + 4, rel + 4 + orig.slen);
      }
      newStrBuf.writeUInt32LE(cursor, 20 + id * 4);
      const lenBuf = Buffer.alloc(4);
      lenBuf.writeInt32LE(payload.length, 0);
      chunks.push(lenBuf, payload);
      cursor += 4 + payload.length;
    }

    const used = cursor - spanStartAbs;
    if (used > capacity) {
      throw new Error(`Span ${sp.name} overflowed by ${used - capacity} bytes (used=${used}, capacity=${capacity})`);
    }
    const packed = Buffer.concat(chunks);
    packed.copy(newStrBuf, spanStartAbs - strSecOff);
    // Zero-pad slack bytes inside this span
    if (used < capacity) {
      newStrBuf.fill(0, (spanStartAbs - strSecOff) + used, (spanEndAbs - strSecOff));
    }
    console.log(`Repacked span ${sp.name} [#${sp.start}..#${sp.end}]: modified=${spanIds[sIdx].length}, used=${used}/${capacity} bytes (free slack=${capacity - used} bytes)`);
  }

  // 2. Any outside-span IDs must fit strictly in-place
  for (const id of inplaceIds) {
    const orig = readStringAt(strBuf, strSecOff, id);
    const payload = Buffer.from(translations.get(id), 'utf8');
    if (payload.length > orig.slen) {
      throw new Error(`Outside-span ID ${id} cannot grow in-place (${payload.length} > ${orig.slen})`);
    }
    const rel = orig.absOff - strSecOff;
    newStrBuf.writeInt32LE(payload.length, rel);
    payload.copy(newStrBuf, rel + 4);
    if (payload.length < orig.slen) {
      newStrBuf.fill(0, rel + 4 + payload.length, rel + 4 + orig.slen);
    }
    replacedCount++;
    totalOrigBytes += orig.slen;
    totalNewBytes += payload.length;
  }

  // 3. Re-encrypt strSec and write full output file
  fs.mkdirSync(path.dirname(outPath), { recursive: true });
  const outBuf = Buffer.from(origBuf);
  const encStrSec = Buffer.from(newStrBuf);
  xorMMapInPlace(encStrSec, strSecOff, key4096);
  encStrSec.copy(outBuf, strSecOff);
  if (outBuf.length !== origSize) throw new Error('Output size mismatch!');
  fs.writeFileSync(outPath, outBuf);

  // 4. Full round-trip verification of ALL strings on disk!
  console.log('Running full round-trip verification of all', count, 'strings...');
  const verifyFile = fs.readFileSync(outPath);
  const vMeta = parseVStringMeta(verifyFile, key4096);
  if (vMeta.count !== count) throw new Error('Verified count mismatch');

  let verifiedTranslated = 0;
  let verifiedUnchanged = 0;
  for (let id = 0; id < count; id++) {
    const got = readStringAt(vMeta.strBuf, vMeta.strSecOff, id);
    if (translations.has(id)) {
      const expected = translations.get(id);
      if (got.text !== expected) {
        throw new Error(`Round-trip mismatch on translated ID ${id}: got=${JSON.stringify(got.text)} expected=${JSON.stringify(expected)}`);
      }
      verifiedTranslated++;
    } else {
      const orig = readStringAt(strBuf, strSecOff, id);
      if (got.slen !== orig.slen || got.text !== orig.text) {
        throw new Error(`Round-trip mismatch on untouched ID ${id}`);
      }
      verifiedUnchanged++;
    }
  }

  const originalSha256 = sha256File(originalPath);
  const candidateSha256 = sha256File(outPath);
  const manifest = {
    targetPath,
    originalBackupPath: originalPath,
    candidatePath: outPath,
    patchSourcePath: patchPath,
    fileSize: origSize,
    totalStrings: count,
    replacedStrings: replacedCount,
    originalUtf8Bytes: totalOrigBytes,
    translatedUtf8Bytes: totalNewBytes,
    expectedOriginalSha256: originalSha256,
    candidateSha256,
    verifiedAt: new Date().toISOString()
  };
  fs.mkdirSync(path.dirname(manifestOut), { recursive: true });
  fs.writeFileSync(manifestOut, JSON.stringify(manifest, null, 2));

  console.log('VERIFICATION PASSED 100%!');
  console.log({ verifiedTranslated, verifiedUnchanged, originalSha256, candidateSha256, fileSize: origSize });
}

main();
