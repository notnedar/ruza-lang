import crypto from 'node:crypto';

export const UNITYCN_KEY = Buffer.from('z4l23bh5#35h&321', 'ascii');
export const UNITYCN_SIG = '#$manjuuunity*!@';

function aesEcbEncrypt16(key, block) {
  const c = crypto.createCipheriv('aes-128-ecb', key, null);
  c.setAutoPadding(false);
  return Buffer.concat([c.update(block), c.final()]);
}

function xor16(a, b) {
  const out = Buffer.alloc(16);
  for (let i = 0; i < 16; i++) out[i] = a[i] ^ b[i];
  return out;
}

export function lz4Decompress(src, dstLen) {
  const dst = Buffer.alloc(dstLen);
  let sp = 0;
  let dp = 0;
  while (sp < src.length) {
    const token = src[sp++];
    let litLen = token >>> 4;
    if (litLen === 15) {
      let b;
      do {
        b = src[sp++];
        litLen += b;
      } while (b === 255);
    }
    src.copy(dst, dp, sp, sp + litLen);
    sp += litLen;
    dp += litLen;
    if (sp >= src.length) break;
    const offset = src[sp] | (src[sp + 1] << 8);
    sp += 2;
    let matchLen = (token & 0x0f) + 4;
    if ((token & 0x0f) === 15) {
      let b;
      do {
        b = src[sp++];
        matchLen += b;
      } while (b === 255);
    }
    for (let i = 0; i < matchLen; i++) {
      dst[dp + i] = dst[dp - offset + i];
    }
    dp += matchLen;
  }
  if (dp !== dstLen) {
    throw new Error(`LZ4 decompress length mismatch: got ${dp}, expected ${dstLen}`);
  }
  return dst;
}

/**
 * LZ4HC-style block compressor (hash chain + lazy matching) with optional `maxMatches` limit.
 * Achieves compression ratios matching Unity's LZ4HC (flags 0x03).
 */
export function lz4Compress(src, maxMatches = Infinity, chainDepth = 256) {
  const n = src.length;
  const out = [];
  const head = new Int32Array(1 << 16).fill(-1);
  const prev = new Int32Array(n).fill(-1);
  let anchor = 0;
  let ip = 0;
  const mflimit = n - 12;
  const matchlimit = n - 5;
  let matchesUsed = 0;

  const insert = (pos) => {
    const v = src.readUInt32LE(pos);
    const h = (Math.imul(v, 2654435761) >>> 16) & 0xffff;
    prev[pos] = head[h];
    head[h] = pos;
  };

  const findBest = (pos) => {
    const v = src.readUInt32LE(pos);
    const h = (Math.imul(v, 2654435761) >>> 16) & 0xffff;
    let ref = head[h];
    let bestLen = 3;
    let bestOff = 0;
    let steps = 0;
    while (ref >= 0 && pos - ref <= 65535 && steps++ < chainDepth) {
      if (src[ref + bestLen] === src[pos + bestLen] && src.readUInt32LE(ref) === v) {
        let ml = 4;
        while (pos + ml < matchlimit && src[ref + ml] === src[pos + ml]) ml++;
        if (ml > bestLen) {
          bestLen = ml;
          bestOff = pos - ref;
          if (pos + ml >= matchlimit) break;
        }
      }
      ref = prev[ref];
    }
    return bestLen >= 4 ? { len: bestLen, off: bestOff } : null;
  };

  const emitSequence = (litStart, litLen, offset, matchLen) => {
    const tokenLit = litLen >= 15 ? 15 : litLen;
    const tokenMatch = matchLen > 0 ? (matchLen - 4 >= 15 ? 15 : matchLen - 4) : 0;
    out.push((tokenLit << 4) | tokenMatch);
    if (litLen >= 15) {
      let rem = litLen - 15;
      while (rem >= 255) { out.push(255); rem -= 255; }
      out.push(rem);
    }
    for (let i = 0; i < litLen; i++) out.push(src[litStart + i]);
    if (matchLen > 0) {
      out.push(offset & 0xff, (offset >>> 8) & 0xff);
      if (matchLen - 4 >= 15) {
        let rem = matchLen - 4 - 15;
        while (rem >= 255) { out.push(255); rem -= 255; }
        out.push(rem);
      }
    }
  };

  while (ip < mflimit && matchesUsed < maxMatches) {
    const m = findBest(ip);
    insert(ip);
    if (m) {
      // Lazy match check at ip + 1
      if (ip + 1 < mflimit) {
        const m2 = findBest(ip + 1);
        if (m2 && m2.len > m.len + 1) {
          ip++;
          continue;
        }
      }
      emitSequence(anchor, ip - anchor, m.off, m.len);
      for (let k = 1; k < m.len && ip + k < mflimit; k++) insert(ip + k);
      ip += m.len;
      anchor = ip;
      matchesUsed++;
    } else {
      ip++;
    }
  }
  emitSequence(anchor, n - anchor, 0, 0);
  return Buffer.from(out);
}

/**
 * Compress `src` with standard LZ4HC to an EXACT target compressed length `targetSize`
 * (requires minCompressed <= targetSize <= rawLiteralEncodedSize).
 */
export function lz4CompressExact(src, targetSize) {
  const n = src.length;
  const head = new Int32Array(1 << 16).fill(-1);
  const prev = new Int32Array(n).fill(-1);
  const mflimit = n - 12;
  const matchlimit = n - 5;
  const chainDepth = 1024;

  const insert = (pos) => {
    const v = src.readUInt32LE(pos);
    const h = (Math.imul(v, 2654435761) >>> 16) & 0xffff;
    prev[pos] = head[h];
    head[h] = pos;
  };
  const findBest = (pos) => {
    const v = src.readUInt32LE(pos);
    const h = (Math.imul(v, 2654435761) >>> 16) & 0xffff;
    let ref = head[h];
    let bestLen = 3, bestOff = 0, steps = 0;
    while (ref >= 0 && pos - ref <= 65535 && steps++ < chainDepth) {
      if (src[ref + bestLen] === src[pos + bestLen] && src.readUInt32LE(ref) === v) {
        let ml = 4;
        while (pos + ml < matchlimit && src[ref + ml] === src[pos + ml]) ml++;
        if (ml > bestLen) {
          bestLen = ml; bestOff = pos - ref;
          if (pos + ml >= matchlimit) break;
        }
      }
      ref = prev[ref];
    }
    return bestLen >= 4 ? { len: bestLen, off: bestOff } : null;
  };

  // Compute all HC sequences once (with 2-step lazy matching like real LZ4HC)!
  const allSeqs = [];
  let anchor = 0, ip = 0;
  while (ip < mflimit) {
    const m = findBest(ip);
    insert(ip);
    if (m) {
      // Check ip+1 for strictly longer match (LZ4HC lazy parse)
      if (ip + 1 < mflimit) {
        const m2 = findBest(ip + 1);
        if (m2 && m2.len > m.len) {
          ip++;
          continue;
        }
      }
      allSeqs.push({ litStart: anchor, litLen: ip - anchor, offset: m.off, matchLen: m.len });
      for (let k = 1; k < m.len && ip + k < mflimit; k++) insert(ip + k);
      ip += m.len;
      anchor = ip;
    } else {
      ip++;
    }
  }

  const encodeSeqs = (list, tailStart) => {
    const out = [];
    const emit = (ls, ll, off, ml) => {
      const tl = ll >= 15 ? 15 : ll;
      const tm = ml > 0 ? (ml - 4 >= 15 ? 15 : ml - 4) : 0;
      out.push((tl << 4) | tm);
      if (ll >= 15) { let r = ll - 15; while (r >= 255) { out.push(255); r -= 255; } out.push(r); }
      for (let i = 0; i < ll; i++) out.push(src[ls + i]);
      if (ml > 0) {
        out.push(off & 0xff, (off >>> 8) & 0xff);
        if (ml - 4 >= 15) { let r = ml - 4 - 15; while (r >= 255) { out.push(255); r -= 255; } out.push(r); }
      }
    };
    for (const s of list) emit(s.litStart, s.litLen, s.offset, s.matchLen);
    emit(tailStart, n - tailStart, 0, 0);
    return Buffer.from(out);
  };

  const fullCompressed = encodeSeqs(allSeqs, anchor);
  if (fullCompressed.length === targetSize) return fullCompressed;
  if (fullCompressed.length > targetSize) {
    throw new Error(`Cannot fit LZ4HC block into targetSize=${targetSize} (min compressed=${fullCompressed.length})`);
  }
  const rawOnly = encodeSeqs([], 0);
  if (rawOnly.length < targetSize) {
    throw new Error(`Uncompressed LZ4 literal size (${rawOnly.length}) is smaller than targetSize=${targetSize}`);
  }

  // Binary search prefix count of allSeqs!
  const tailFor = (k) => (k === 0 ? 0 : allSeqs[k - 1].litStart + allSeqs[k - 1].litLen + allSeqs[k - 1].matchLen);
  let lo = 0, hi = allSeqs.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    const len = encodeSeqs(allSeqs.slice(0, mid), tailFor(mid)).length;
    if (len <= targetSize) hi = mid;
    else lo = mid + 1;
  }

  const seqs = allSeqs.slice(0, lo).map(s => ({ ...s }));
  let tailStart = tailFor(lo);
  let curBuf = encodeSeqs(seqs, tailStart);
  if (curBuf.length === targetSize) return curBuf;

  // Fine-tune by shrinking matches from the end by 1 byte at a time!
  if (seqs.length > 0) {
    const last = seqs[seqs.length - 1];
    while (last.matchLen > 4 && curBuf.length < targetSize) {
      last.matchLen -= 1;
      tailStart -= 1;
      const nextBuf = encodeSeqs(seqs, tailStart);
      if (nextBuf.length <= targetSize) curBuf = nextBuf;
      else { last.matchLen += 1; tailStart += 1; break; }
    }
  }
  for (let j = seqs.length - 2; j >= 0 && curBuf.length < targetSize; j--) {
    while (seqs[j].matchLen > 4 && curBuf.length < targetSize) {
      seqs[j].matchLen -= 1;
      seqs[j + 1].litStart -= 1;
      seqs[j + 1].litLen += 1;
      const nextBuf = encodeSeqs(seqs, tailStart);
      if (nextBuf.length <= targetSize) curBuf = nextBuf;
      else {
        seqs[j].matchLen += 1;
        seqs[j + 1].litStart += 1;
        seqs[j + 1].litLen -= 1;
        break;
      }
    }
  }
  if (curBuf.length !== targetSize) {
    throw new Error(`Could not tune LZ4HC to exact size ${targetSize} (got ${curBuf.length})`);
  }
  const check = lz4Decompress(curBuf, n);
  if (!check.equals(src)) throw new Error('Exact LZ4HC round-trip failed');
  return curBuf;
}

export function createUnityCNCipher(infoBytes, infoKey) {
  const dec16 = xor16(aesEcbEncrypt16(UNITYCN_KEY, infoKey), infoBytes);
  const index = new Uint8Array(16);
  const invIndex = new Uint8Array(16);
  for (let i = 0; i < 8; i++) {
    index[2 * i] = dec16[i] >> 4;
    index[2 * i + 1] = dec16[i] & 0xf;
  }
  for (let i = 0; i < 16; i++) invIndex[index[i]] = i;

  const S = new Uint8Array(16);
  for (let i = 0; i < 4; i++) {
    const b0 = dec16[8 + 2 * i];
    const b1 = dec16[8 + 2 * i + 1];
    S[0 + i] = b0 >> 4;
    S[4 + i] = b0 & 0xf;
    S[8 + i] = b1 >> 4;
    S[12 + i] = b1 & 0xf;
  }

  const r9At = (idx) =>
    (S[12 + ((idx >> 6) & 3)] + S[8 + ((idx >> 4) & 3)] + S[4 + ((idx >> 2) & 3)] + S[idx & 3]) & 0xff;

  function decryptByte(buf, off, idx) {
    const b = buf[off];
    const r9 = r9At(idx);
    const out = (((index[b >> 4] - r9) & 0xf) << 4) | ((index[b & 0xf] - r9) & 0xf);
    buf[off] = out;
    return out;
  }

  function encryptByte(buf, off, idx) {
    const p = buf[off];
    const r9 = r9At(idx);
    const out = (invIndex[((p >> 4) + r9) & 0xf] << 4) | invIndex[((p & 0xf) + r9) & 0xf];
    buf[off] = out;
    return p; // returns original plaintext byte for LZ4 parsing!
  }

  function decryptBlockInPlace(buf, blockIndex) {
    let p = 0;
    let seqIdx = blockIndex;
    while (p < buf.length) {
      let bIdx = seqIdx++;
      const token = decryptByte(buf, p++, bIdx++);
      let litLen = token >>> 4;
      if (litLen === 15) {
        let extra;
        do { extra = decryptByte(buf, p++, bIdx++); litLen += extra; } while (extra === 255);
      }
      p += litLen;
      if (p >= buf.length) break;
      decryptByte(buf, p++, bIdx++);
      decryptByte(buf, p++, bIdx++);
      if ((token & 0xf) === 15) {
        let extra;
        do { extra = decryptByte(buf, p++, bIdx++); } while (extra === 255);
      }
    }
  }

  function encryptBlockInPlace(buf, blockIndex) {
    let p = 0;
    let seqIdx = blockIndex;
    while (p < buf.length) {
      let bIdx = seqIdx++;
      const token = encryptByte(buf, p++, bIdx++);
      let litLen = token >>> 4;
      if (litLen === 15) {
        let extra;
        do { extra = encryptByte(buf, p++, bIdx++); litLen += extra; } while (extra === 255);
      }
      p += litLen;
      if (p >= buf.length) break;
      encryptByte(buf, p++, bIdx++);
      encryptByte(buf, p++, bIdx++);
      if ((token & 0xf) === 15) {
        let extra;
        do { extra = encryptByte(buf, p++, bIdx++); } while (extra === 255);
      }
    }
  }

  return { decryptBlockInPlace, encryptBlockInPlace };
}

export function parseUnityCNBundle(sub) {
  let o = 0;
  const readCStr = () => {
    const e = sub.indexOf(0, o);
    const s = sub.subarray(o, e).toString('utf8');
    o = e + 1;
    return s;
  };
  const sig = readCStr();
  const ver = sub.readUInt32BE(o); o += 4;
  const pver = readCStr();
  const ever = readCStr();
  const fsizeOff = o;
  const fsize = Number(sub.readBigInt64BE(o)); o += 8;
  const cInfoOff = o;
  const cInfo = sub.readUInt32BE(o); o += 4;
  const uInfo = sub.readUInt32BE(o); o += 4;
  const flags = sub.readUInt32BE(o); o += 4;
  if (ver >= 7) o = (o + 15) & ~15;

  const encHdrOff = o;
  const infoBytes = Buffer.from(sub.subarray(o, o + 16));
  const infoKey = Buffer.from(sub.subarray(o + 16, o + 32));
  const sigBytes = Buffer.from(sub.subarray(o + 33, o + 49));
  const sigKey = Buffer.from(sub.subarray(o + 49, o + 65));
  const checkSig = xor16(aesEcbEncrypt16(UNITYCN_KEY, sigKey), sigBytes).toString('ascii');
  if (checkSig !== UNITYCN_SIG) throw new Error(`Invalid UnityCN signature: ${JSON.stringify(checkSig)}`);

  const cipher = createUnityCNCipher(infoBytes, infoKey);
  const blocksInfoOff = (o + 65 + 15) & ~15;
  const blocksInfo = lz4Decompress(sub.subarray(blocksInfoOff, blocksInfoOff + cInfo), uInfo);
  const dataStart = (blocksInfoOff + cInfo + 15) & ~15;

  let bp = 0;
  const dataHash = Buffer.from(blocksInfo.subarray(0, 16)); bp = 16;
  const bCount = blocksInfo.readInt32BE(bp); bp += 4;
  const blocks = [];
  let curDataOff = dataStart;
  for (let i = 0; i < bCount; i++) {
    const uSize = blocksInfo.readUInt32BE(bp);
    const cSize = blocksInfo.readUInt32BE(bp + 4);
    const bFlags = blocksInfo.readUInt16BE(bp + 8);
    blocks.push({ index: i, uSize, cSize, flags: bFlags, fileOff: curDataOff });
    curDataOff += cSize;
    bp += 10;
  }
  const dCount = blocksInfo.readInt32BE(bp); bp += 4;
  const dirs = [];
  for (let i = 0; i < dCount; i++) {
    const off = Number(blocksInfo.readBigInt64BE(bp)); bp += 8;
    const size = Number(blocksInfo.readBigInt64BE(bp)); bp += 8;
    const fl = blocksInfo.readUInt32BE(bp); bp += 4;
    const end = blocksInfo.indexOf(0, bp);
    const name = blocksInfo.subarray(bp, end).toString('utf8'); bp = end + 1;
    dirs.push({ off, size, flags: fl, name });
  }

  return {
    sig, ver, pver, ever, fsize, fsizeOff, cInfo, cInfoOff, uInfo, flags,
    encHdrOff, blocksInfoOff, dataStart, dataHash, blocks, dirs, cipher,
    headerPrefix: Buffer.from(sub.subarray(0, blocksInfoOff)),
    readBlock(i) {
      const blk = blocks[i];
      const cbuf = Buffer.from(sub.subarray(blk.fileOff, blk.fileOff + blk.cSize));
      if (blk.flags & 0x100) cipher.decryptBlockInPlace(cbuf, i);
      const ct = blk.flags & 0x3f;
      return ct === 0 ? cbuf : lz4Decompress(cbuf, blk.uSize);
    },
    readAll() {
      const chunks = [];
      for (let i = 0; i < blocks.length; i++) chunks.push(this.readBlock(i));
      return Buffer.concat(chunks);
    },
  };
}
