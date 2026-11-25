import { fileURLToPath } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { parseUnityCNBundle, lz4Compress, lz4CompressExact } from './unitycn-lib.mjs';

const WORKSPACE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const SUB_OFFSET = 420062;
const SUB_SIZE = 83151;
const DATA_OFF = 3392;
const MB_NAME_OFF = 3420;
const OBJ0_SIZE_OFF = 3220;
const ORIG_MB_SIZE = 161392;

function sha256Buf(buf) {
  return crypto.createHash('sha256').update(buf).digest('hex').toUpperCase();
}

function align4(n) {
  return (n + 3) & ~3;
}

export function readStaticTextPairs(sf) {
  let cur = MB_NAME_OFF + 28; // right after len(23) + "ast_staticTextKeyConfig\0"
  const langLen = sf.readInt32LE(cur); cur += 4;
  const lang = sf.subarray(cur, cur + langLen).toString('utf8');
  cur = align4(cur + langLen);
  const count = sf.readInt32LE(cur); cur += 4;
  const pairs = [];
  for (let i = 0; i < count; i++) {
    const kl = sf.readInt32LE(cur); cur += 4;
    const key = sf.subarray(cur, cur + kl).toString('utf8');
    cur = align4(cur + kl);
    const vl = sf.readInt32LE(cur); cur += 4;
    const val = sf.subarray(cur, cur + vl).toString('utf8');
    cur = align4(cur + vl);
    pairs.push({ idx: i, key, val });
  }
  return { lang, count, pairs, endCur: cur };
}

export function buildStaticTextSerializedFile(origSf, translationsByKeyOrIdx) {
  const { lang, count, pairs } = readStaticTextPairs(origSf);
  const parts = [];
  // Copy MonoBehaviour header from DATA_OFF up to MB_NAME_OFF + 28 (exact prefix including PPtrs + m_Name)
  parts.push(origSf.subarray(DATA_OFF, MB_NAME_OFF + 28));

  const pushStr = (s) => {
    const b = Buffer.from(s, 'utf8');
    const hdr = Buffer.alloc(4);
    hdr.writeInt32LE(b.length, 0);
    const padLen = align4(b.length) - b.length;
    parts.push(hdr, b);
    if (padLen > 0) parts.push(Buffer.alloc(padLen));
  };

  pushStr(lang);
  const cntBuf = Buffer.alloc(4);
  cntBuf.writeInt32LE(count, 0);
  parts.push(cntBuf);

  let replaced = 0;
  let origBytes = 0;
  let newBytes = 0;
  for (const p of pairs) {
    let val = p.val;
    const tr = translationsByKeyOrIdx.get(p.idx) ?? translationsByKeyOrIdx.get(p.key);
    if (typeof tr === 'string') {
      origBytes += Buffer.byteLength(p.val, 'utf8');
      newBytes += Buffer.byteLength(tr, 'utf8');
      val = tr;
      replaced++;
    }
    pushStr(p.key);
    pushStr(val);
  }

  const newMb = Buffer.concat(parts);
  if (newMb.length > ORIG_MB_SIZE) {
    throw new Error(`Rebuilt MonoBehaviour exceeds original slot (${newMb.length} > ${ORIG_MB_SIZE})`);
  }

  const newSf = Buffer.from(origSf);
  newSf.fill(0, DATA_OFF, DATA_OFF + ORIG_MB_SIZE);
  newMb.copy(newSf, DATA_OFF);
  // Update Object #0 byteSize in SerializedFile metadata (Object #1 stays untouched at byteStart=161392!)
  newSf.writeUInt32LE(newMb.length, OBJ0_SIZE_OFF);
  return { newSf, newMbSize: newMb.length, replaced, origBytes, newBytes };
}

function main() {
  const args = process.argv.slice(2);
  const getArg = (f, d) => { const i = args.indexOf(f); return i >= 0 && i + 1 < args.length ? args[i + 1] : d; };

  const origPath = getArg('--original', path.join(WORKSPACE, 'backups/yr49bxop0f3reujfsmyfca.r.original'));
  const patchPath = getArg('--patch', path.join(WORKSPACE, 'patches/static-ui-en-3219598.json'));
  const outPath = getArg('--out', path.join(WORKSPACE, 'work/yr49bxop0f3reujfsmyfca.r.english'));

  const patchObj = JSON.parse(fs.readFileSync(patchPath, 'utf8'));
  const trMap = new Map();
  for (const [k, v] of Object.entries(patchObj)) {
    if (k.startsWith('_')) continue;
    trMap.set(k, v);
  }

  const origBuf = fs.readFileSync(origPath);
  const sub = origBuf.subarray(SUB_OFFSET, SUB_OFFSET + SUB_SIZE);
  const bundle = parseUnityCNBundle(sub);
  const origSf = bundle.readAll();

  const { newSf, newMbSize, replaced, origBytes, newBytes } = buildStaticTextSerializedFile(origSf, trMap);
  console.log(`Rebuilt ast_staticTextKeyConfig.asset: replaced=${replaced}, mbSize=${newMbSize}/${ORIG_MB_SIZE} (saved ${origBytes - newBytes} UTF-8 bytes)`);

  // Split into original 2 block sizes (131072 + 34056)
  const totalDataBudget = SUB_SIZE - bundle.dataStart; // 83151 - 208 = 82943
  const blk0Plain = newSf.subarray(0, bundle.blocks[0].uSize);
  const blk1Plain = newSf.subarray(bundle.blocks[0].uSize, bundle.blocks[0].uSize + bundle.blocks[1].uSize);

  const c0 = lz4Compress(blk0Plain);
  const blk1Target = totalDataBudget - c0.length;
  const c1 = lz4CompressExact(blk1Plain, blk1Target);
  bundle.cipher.encryptBlockInPlace(c0, 0);
  bundle.cipher.encryptBlockInPlace(c1, 1);

  // Rebuild blocksInfo with updated cSize0 and cSize1 (keeps dataStart=208 and exact SUB_SIZE=83151!)
  const biParts = [];
  biParts.push(bundle.dataHash); // 16B
  const bCnt = Buffer.alloc(4); bCnt.writeInt32BE(2, 0); biParts.push(bCnt);
  for (const [uSz, cSz, fl] of [
    [bundle.blocks[0].uSize, c0.length, bundle.blocks[0].flags],
    [bundle.blocks[1].uSize, c1.length, bundle.blocks[1].flags],
  ]) {
    const bb = Buffer.alloc(10);
    bb.writeUInt32BE(uSz, 0); bb.writeUInt32BE(cSz, 4); bb.writeUInt16BE(fl, 8);
    biParts.push(bb);
  }
  const dCnt = Buffer.alloc(4); dCnt.writeInt32BE(bundle.dirs.length, 0); biParts.push(dCnt);
  for (const d of bundle.dirs) {
    const db = Buffer.alloc(20);
    db.writeBigInt64BE(BigInt(d.off), 0);
    db.writeBigInt64BE(BigInt(d.size), 8);
    db.writeUInt32BE(d.flags, 16);
    biParts.push(db, Buffer.from(d.name, 'utf8'), Buffer.from([0]));
  }
  const newBlocksInfoPlain = Buffer.concat(biParts);
  const newBlocksInfoComp = lz4Compress(newBlocksInfoPlain);
  if (bundle.blocksInfoOff + newBlocksInfoComp.length > bundle.dataStart) {
    throw new Error(`Compressed blocksInfo overflowed dataStart (${bundle.blocksInfoOff + newBlocksInfoComp.length} > ${bundle.dataStart})`);
  }

  const newSub = Buffer.alloc(SUB_SIZE, 0);
  bundle.headerPrefix.copy(newSub, 0);
  newSub.writeBigInt64BE(BigInt(SUB_SIZE), bundle.fsizeOff);
  newSub.writeUInt32BE(newBlocksInfoComp.length, bundle.cInfoOff);
  newSub.writeUInt32BE(newBlocksInfoPlain.length, bundle.cInfoOff + 4);
  newBlocksInfoComp.copy(newSub, bundle.blocksInfoOff);
  c0.copy(newSub, bundle.dataStart);
  c1.copy(newSub, bundle.dataStart + c0.length);

  const outBuf = Buffer.from(origBuf);
  newSub.copy(outBuf, SUB_OFFSET);
  if (outBuf.length !== origBuf.length) throw new Error('Total file size mismatch!');
  fs.writeFileSync(outPath, outBuf);

  // Full round-trip verification of ALL 3,573 entries!
  const verifyBuf = fs.readFileSync(outPath);
  const vBundle = parseUnityCNBundle(verifyBuf.subarray(SUB_OFFSET, SUB_OFFSET + SUB_SIZE));
  const vSf = vBundle.readAll();
  if (!vSf.equals(newSf)) throw new Error('Round-trip decrypted SerializedFile mismatch!');
  const vParsed = readStaticTextPairs(vSf);
  const RE_CJK = /[\u3040-\u309f\u30a0-\u30ff\u4e00-\u9fff]/;
  const leftover = vParsed.pairs.filter(p => RE_CJK.test(p.val));
  if (leftover.length > 0) {
    console.warn(`Warning: ${leftover.length} entries still have CJK`, leftover.slice(0, 5));
  }
  console.log('ast_staticTextKeyConfig.asset PATCH VERIFIED 100%!', {
    totalPairs: vParsed.count,
    replaced,
    leftoverCjk: leftover.length,
    fileSize: verifyBuf.length,
    origSha256: sha256Buf(origBuf),
    candidateSha256: sha256Buf(verifyBuf),
  });
}

if (process.argv[1] && process.argv[1].endsWith('static-text-patcher.mjs')) {
  main();
}
