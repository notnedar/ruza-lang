use aes::cipher::{BlockEncrypt, KeyInit};
use aes::Aes128;
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub const UNITYCN_KEY: &[u8; 16] = b"z4l23bh5#35h&321";
pub const UNITYCN_SIG: &[u8; 16] = b"#$manjuuunity*!@";

pub fn gunzip(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut dec = GzDecoder::new(data);
    let mut out = Vec::new();
    dec.read_to_end(&mut out)
        .map_err(|e| format!("GZip decompress failed: {}", e))?;
    Ok(out)
}

pub fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex_upper(&hasher.finalize())
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut f = File::open(path).map_err(|e| format!("Open {:?} failed: {}", path, e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 4 * 1024 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("Read {:?} failed: {}", path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_upper(&hasher.finalize()))
}

fn hex_upper(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xF) as usize] as char);
    }
    s
}

pub fn xor_mmap_in_place(buf: &mut [u8], file_start_offset: usize, key4096: &[u8]) {
    for (i, byte) in buf.iter_mut().enumerate() {
        let pos = file_start_offset + i;
        let shift = key4096[(pos >> 12) & 0xFF] as usize;
        *byte ^= key4096[((pos & 0xFFF) + shift) & 0xFFF];
    }
}

#[inline]
pub fn read_u32_le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
#[inline]
pub fn read_i32_le(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
#[inline]
pub fn write_u32_le(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
#[inline]
pub fn write_i32_le(b: &mut [u8], o: usize, v: i32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
#[inline]
pub fn read_u16_be(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}
#[inline]
pub fn read_u32_be(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
#[inline]
pub fn read_i64_be(b: &[u8], o: usize) -> i64 {
    i64::from_be_bytes([
        b[o], b[o + 1], b[o + 2], b[o + 3], b[o + 4], b[o + 5], b[o + 6], b[o + 7],
    ])
}
#[allow(dead_code)]
#[inline]
pub fn write_u16_be(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_be_bytes());
}
#[inline]
pub fn write_u32_be(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_be_bytes());
}
#[inline]
pub fn write_i64_be(b: &mut [u8], o: usize, v: i64) {
    b[o..o + 8].copy_from_slice(&v.to_be_bytes());
}

fn aes_ecb_encrypt16(key: &[u8; 16], block: &[u8]) -> [u8; 16] {
    let cipher = Aes128::new_from_slice(key).expect("AES-128 key");
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&block[..16]);
    let mut blk = aes::Block::from(arr);
    cipher.encrypt_block(&mut blk);
    let mut out = [0u8; 16];
    out.copy_from_slice(&blk);
    out
}

fn xor16(a: &[u8], b: &[u8]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = a[i] ^ b[i];
    }
    out
}

pub fn lz4_decompress(src: &[u8], dst_len: usize) -> Result<Vec<u8>, String> {
    let mut dst = vec![0u8; dst_len];
    let mut sp = 0usize;
    let mut dp = 0usize;
    while sp < src.len() {
        let token = src[sp] as usize;
        sp += 1;
        let mut lit_len = token >> 4;
        if lit_len == 15 {
            loop {
                if sp >= src.len() {
                    return Err("LZ4 truncated literal length".into());
                }
                let b = src[sp] as usize;
                sp += 1;
                lit_len += b;
                if b != 255 {
                    break;
                }
            }
        }
        if sp + lit_len > src.len() || dp + lit_len > dst_len {
            return Err("LZ4 literal overflow".into());
        }
        dst[dp..dp + lit_len].copy_from_slice(&src[sp..sp + lit_len]);
        sp += lit_len;
        dp += lit_len;
        if sp >= src.len() {
            break;
        }
        if sp + 2 > src.len() {
            return Err("LZ4 truncated offset".into());
        }
        let offset = (src[sp] as usize) | ((src[sp + 1] as usize) << 8);
        sp += 2;
        if offset == 0 || offset > dp {
            return Err("LZ4 invalid match offset".into());
        }
        let mut match_len = (token & 0x0F) + 4;
        if (token & 0x0F) == 15 {
            loop {
                if sp >= src.len() {
                    return Err("LZ4 truncated match length".into());
                }
                let b = src[sp] as usize;
                sp += 1;
                match_len += b;
                if b != 255 {
                    break;
                }
            }
        }
        if dp + match_len > dst_len {
            return Err("LZ4 match overflow".into());
        }
        for i in 0..match_len {
            dst[dp + i] = dst[dp - offset + i];
        }
        dp += match_len;
    }
    if dp != dst_len {
        return Err(format!("LZ4 decompress size mismatch: {} != {}", dp, dst_len));
    }
    Ok(dst)
}

#[derive(Clone, Copy)]
struct Lz4Seq {
    lit_start: usize,
    lit_len: usize,
    offset: usize,
    match_len: usize,
}

#[inline]
fn insert_hash(src: &[u8], pos: usize, head: &mut [i32], prev: &mut [i32]) {
    let v = read_u32_le(src, pos);
    let h = (v.wrapping_mul(2654435761u32) >> 16) as usize;
    prev[pos] = head[h];
    head[h] = pos as i32;
}

#[inline]
fn find_best_match(
    src: &[u8],
    pos: usize,
    matchlimit: usize,
    head: &[i32],
    prev: &[i32],
    chain_depth: usize,
) -> (usize, usize) {
    let v = read_u32_le(src, pos);
    let h = (v.wrapping_mul(2654435761u32) >> 16) as usize;
    let mut r = head[h];
    let mut best_len = 3usize;
    let mut best_off = 0usize;
    let mut steps = 0usize;
    while r >= 0 && steps < chain_depth {
        steps += 1;
        let ru = r as usize;
        let dist = pos - ru;
        if dist > 65535 {
            break;
        }
        if src[ru + best_len] == src[pos + best_len] && read_u32_le(src, ru) == v {
            let mut ml = 4usize;
            while pos + ml < matchlimit && src[ru + ml] == src[pos + ml] {
                ml += 1;
            }
            if ml > best_len {
                best_len = ml;
                best_off = dist;
                if pos + ml >= matchlimit {
                    break;
                }
            }
        }
        r = prev[ru];
    }
    (best_len, best_off)
}

fn find_hc_sequences(src: &[u8], chain_depth: usize) -> (Vec<Lz4Seq>, usize) {
    let n = src.len();
    if n < 13 {
        return (Vec::new(), 0);
    }
    let mut head = vec![-1i32; 1 << 16];
    let mut prev = vec![-1i32; n];
    let mflimit = n - 12;
    let matchlimit = n - 5;
    let mut seqs = Vec::with_capacity(n / 16 + 16);
    let mut anchor = 0usize;
    let mut ip = 0usize;

    while ip < mflimit {
        let (best_len, best_off) = find_best_match(src, ip, matchlimit, &head, &prev, chain_depth);
        insert_hash(src, ip, &mut head, &mut prev);
        if best_len >= 4 {
            if ip + 1 < mflimit {
                let (b2_len, _) = find_best_match(src, ip + 1, matchlimit, &head, &prev, chain_depth);
                if b2_len > best_len {
                    ip += 1;
                    continue;
                }
            }
            seqs.push(Lz4Seq {
                lit_start: anchor,
                lit_len: ip - anchor,
                offset: best_off,
                match_len: best_len,
            });
            for k in 1..best_len {
                if ip + k < mflimit {
                    insert_hash(src, ip + k, &mut head, &mut prev);
                }
            }
            ip += best_len;
            anchor = ip;
        } else {
            ip += 1;
        }
    }
    (seqs, anchor)
}

fn measure_encoded_size(seqs: &[Lz4Seq], tail_start: usize, total_len: usize) -> usize {
    let mut sz = 0usize;
    for s in seqs {
        sz += 1 + s.lit_len + 2;
        if s.lit_len >= 15 {
            sz += ((s.lit_len - 15) / 255) + 1;
        }
        if s.match_len - 4 >= 15 {
            sz += ((s.match_len - 19) / 255) + 1;
        }
    }
    let tail_len = total_len - tail_start;
    sz += 1 + tail_len;
    if tail_len >= 15 {
        sz += ((tail_len - 15) / 255) + 1;
    }
    sz
}

fn encode_sequences(src: &[u8], seqs: &[Lz4Seq], tail_start: usize) -> Vec<u8> {
    let total = measure_encoded_size(seqs, tail_start, src.len());
    let mut out = Vec::with_capacity(total);
    for s in seqs {
        let tl = s.lit_len.min(15);
        let tm = (s.match_len - 4).min(15);
        out.push(((tl << 4) | tm) as u8);
        if s.lit_len >= 15 {
            let mut rem = s.lit_len - 15;
            while rem >= 255 {
                out.push(255);
                rem -= 255;
            }
            out.push(rem as u8);
        }
        out.extend_from_slice(&src[s.lit_start..s.lit_start + s.lit_len]);
        out.push((s.offset & 0xFF) as u8);
        out.push(((s.offset >> 8) & 0xFF) as u8);
        if s.match_len - 4 >= 15 {
            let mut rem = s.match_len - 19;
            while rem >= 255 {
                out.push(255);
                rem -= 255;
            }
            out.push(rem as u8);
        }
    }
    let tail_len = src.len() - tail_start;
    let ttl = tail_len.min(15);
    out.push((ttl << 4) as u8);
    if tail_len >= 15 {
        let mut rem = tail_len - 15;
        while rem >= 255 {
            out.push(255);
            rem -= 255;
        }
        out.push(rem as u8);
    }
    out.extend_from_slice(&src[tail_start..]);
    debug_assert_eq!(out.len(), total);
    out
}

pub fn lz4_compress(src: &[u8], chain_depth: usize) -> Vec<u8> {
    let (seqs, anchor) = find_hc_sequences(src, chain_depth);
    encode_sequences(src, &seqs, anchor)
}

pub fn lz4_compress_exact(src: &[u8], target_size: usize, chain_depth: usize) -> Result<Vec<u8>, String> {
    let (all_seqs, anchor) = find_hc_sequences(src, chain_depth);
    let min_size = measure_encoded_size(&all_seqs, anchor, src.len());
    if min_size == target_size {
        return Ok(encode_sequences(src, &all_seqs, anchor));
    }
    if min_size > target_size {
        return Err(format!(
            "Cannot fit LZ4HC block into target_size={} (min={})",
            target_size, min_size
        ));
    }
    let raw_size = measure_encoded_size(&[], 0, src.len());
    if raw_size < target_size {
        return Err(format!(
            "Uncompressed LZ4 literal size ({}) < target_size={}",
            raw_size, target_size
        ));
    }

    let mut lo = 0usize;
    let mut hi = all_seqs.len();
    while lo < hi {
        let mid = (lo + hi) >> 1;
        let t_start = if mid == 0 {
            0
        } else {
            let s = all_seqs[mid - 1];
            s.lit_start + s.lit_len + s.match_len
        };
        let len = measure_encoded_size(&all_seqs[..mid], t_start, src.len());
        if len <= target_size {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }

    let mut seqs = all_seqs[..lo].to_vec();
    let mut tail_start = if lo == 0 {
        0
    } else {
        let s = seqs[lo - 1];
        s.lit_start + s.lit_len + s.match_len
    };
    let mut cur_size = measure_encoded_size(&seqs, tail_start, src.len());

    if !seqs.is_empty() {
        let last_idx = seqs.len() - 1;
        while seqs[last_idx].match_len > 4 && cur_size < target_size {
            seqs[last_idx].match_len -= 1;
            let next_size = measure_encoded_size(&seqs, tail_start - 1, src.len());
            if next_size <= target_size {
                tail_start -= 1;
                cur_size = next_size;
            } else {
                seqs[last_idx].match_len += 1;
                break;
            }
        }
    }

    if seqs.len() >= 2 {
        for j in (0..seqs.len() - 1).rev() {
            if cur_size >= target_size {
                break;
            }
            while seqs[j].match_len > 4 && cur_size < target_size {
                seqs[j].match_len -= 1;
                seqs[j + 1].lit_start -= 1;
                seqs[j + 1].lit_len += 1;
                let next_size = measure_encoded_size(&seqs, tail_start, src.len());
                if next_size <= target_size {
                    cur_size = next_size;
                } else {
                    seqs[j].match_len += 1;
                    seqs[j + 1].lit_start += 1;
                    seqs[j + 1].lit_len -= 1;
                    break;
                }
            }
        }
    }

    if cur_size != target_size {
        return Err(format!(
            "Failed to tune LZ4HC block to exact size {} (got {})",
            target_size, cur_size
        ));
    }
    let buf = encode_sequences(src, &seqs, tail_start);
    let verify = lz4_decompress(&buf, src.len())?;
    if verify != src {
        return Err("Exact LZ4HC round-trip mismatch".into());
    }
    Ok(buf)
}

pub struct UnityCnCipher {
    index: [u8; 16],
    inv_index: [u8; 16],
    s: [u8; 16],
}

impl UnityCnCipher {
    pub fn new(info_bytes: &[u8], info_key: &[u8]) -> Self {
        let dec16 = xor16(&aes_ecb_encrypt16(UNITYCN_KEY, info_key), info_bytes);
        let mut index = [0u8; 16];
        let mut inv_index = [0u8; 16];
        for i in 0..8 {
            index[2 * i] = dec16[i] >> 4;
            index[2 * i + 1] = dec16[i] & 0xF;
        }
        for i in 0..16 {
            inv_index[index[i] as usize] = i as u8;
        }
        let mut s = [0u8; 16];
        for i in 0..4 {
            let b0 = dec16[8 + 2 * i];
            let b1 = dec16[8 + 2 * i + 1];
            s[i] = b0 >> 4;
            s[4 + i] = b0 & 0xF;
            s[8 + i] = b1 >> 4;
            s[12 + i] = b1 & 0xF;
        }
        Self { index, inv_index, s }
    }

    #[inline]
    fn r9_at(&self, idx: usize) -> u8 {
        self.s[12 + ((idx >> 6) & 3)]
            .wrapping_add(self.s[8 + ((idx >> 4) & 3)])
            .wrapping_add(self.s[4 + ((idx >> 2) & 3)])
            .wrapping_add(self.s[idx & 3])
    }

    #[inline]
    fn decrypt_byte(&self, buf: &mut [u8], off: usize, idx: usize) -> u8 {
        let b = buf[off];
        let r9 = self.r9_at(idx);
        let hi = self.index[(b >> 4) as usize].wrapping_sub(r9) & 0xF;
        let lo = self.index[(b & 0xF) as usize].wrapping_sub(r9) & 0xF;
        let res = (hi << 4) | lo;
        buf[off] = res;
        res
    }

    #[inline]
    fn encrypt_byte(&self, buf: &mut [u8], off: usize, idx: usize) -> u8 {
        let p = buf[off];
        let r9 = self.r9_at(idx);
        let hi = self.inv_index[((p >> 4).wrapping_add(r9) & 0xF) as usize];
        let lo = self.inv_index[((p & 0xF).wrapping_add(r9) & 0xF) as usize];
        buf[off] = (hi << 4) | lo;
        p
    }

    pub fn decrypt_block_in_place(&self, buf: &mut [u8], block_index: usize) {
        let mut p = 0usize;
        let mut seq_idx = block_index;
        while p < buf.len() {
            let mut b_idx = seq_idx;
            seq_idx += 1;
            let tok = self.decrypt_byte(buf, p, b_idx);
            p += 1;
            b_idx += 1;
            let mut lit_len = (tok >> 4) as usize;
            if lit_len == 15 {
                loop {
                    let e = self.decrypt_byte(buf, p, b_idx);
                    p += 1;
                    b_idx += 1;
                    lit_len += e as usize;
                    if e != 255 {
                        break;
                    }
                }
            }
            p += lit_len;
            if p >= buf.len() {
                break;
            }
            self.decrypt_byte(buf, p, b_idx);
            p += 1;
            b_idx += 1;
            self.decrypt_byte(buf, p, b_idx);
            p += 1;
            b_idx += 1;
            if (tok & 0xF) == 15 {
                loop {
                    let e = self.decrypt_byte(buf, p, b_idx);
                    p += 1;
                    b_idx += 1;
                    if e != 255 {
                        break;
                    }
                }
            }
        }
    }

    pub fn encrypt_block_in_place(&self, buf: &mut [u8], block_index: usize) {
        let mut p = 0usize;
        let mut seq_idx = block_index;
        while p < buf.len() {
            let mut b_idx = seq_idx;
            seq_idx += 1;
            let tok = self.encrypt_byte(buf, p, b_idx);
            p += 1;
            b_idx += 1;
            let mut lit_len = (tok >> 4) as usize;
            if lit_len == 15 {
                loop {
                    let e = self.encrypt_byte(buf, p, b_idx);
                    p += 1;
                    b_idx += 1;
                    lit_len += e as usize;
                    if e != 255 {
                        break;
                    }
                }
            }
            p += lit_len;
            if p >= buf.len() {
                break;
            }
            self.encrypt_byte(buf, p, b_idx);
            p += 1;
            b_idx += 1;
            self.encrypt_byte(buf, p, b_idx);
            p += 1;
            b_idx += 1;
            if (tok & 0xF) == 15 {
                loop {
                    let e = self.encrypt_byte(buf, p, b_idx);
                    p += 1;
                    b_idx += 1;
                    if e != 255 {
                        break;
                    }
                }
            }
        }
    }
}

pub struct UnityCnBlock {
    pub u_size: usize,
    pub c_size: usize,
    pub flags: u16,
    pub file_offset: usize,
}

pub struct UnityCnDir {
    pub offset: i64,
    pub size: i64,
    pub flags: u32,
    pub name: String,
}

pub struct UnityCnBundle<'a> {
    pub src: &'a [u8],
    pub base_off: usize,
    pub fsize_off: usize,
    pub cinfo_off: usize,
    pub blocks_info_off: usize,
    pub data_start: usize,
    pub header_prefix: Vec<u8>,
    pub data_hash: [u8; 16],
    pub blocks: Vec<UnityCnBlock>,
    pub dirs: Vec<UnityCnDir>,
    pub cipher: UnityCnCipher,
}

impl<'a> UnityCnBundle<'a> {
    pub fn parse(src: &'a [u8], base_off: usize) -> Result<Self, String> {
        let mut o = base_off;
        let read_cstr = |o: &mut usize| -> Result<String, String> {
            let end = src[*o..]
                .iter()
                .position(|&b| b == 0)
                .ok_or("Missing null terminator in UnityFS header")?;
            let s = String::from_utf8_lossy(&src[*o..*o + end]).into_owned();
            *o += end + 1;
            Ok(s)
        };
        let sig = read_cstr(&mut o)?;
        if sig != "UnityFS" {
            return Err(format!("Expected UnityFS, got {}", sig));
        }
        let ver = read_u32_be(src, o);
        o += 4;
        let _pver = read_cstr(&mut o)?;
        let _ever = read_cstr(&mut o)?;
        let fsize_off = o - base_off;
        let _fsize = read_i64_be(src, o);
        o += 8;
        let cinfo_off = o - base_off;
        let c_info = read_u32_be(src, o) as usize;
        o += 4;
        let u_info = read_u32_be(src, o) as usize;
        o += 4;
        o += 4; // flags
        if ver >= 7 {
            o = base_off + (((o - base_off) + 15) & !15);
        }

        let info_bytes = &src[o..o + 16];
        let info_key = &src[o + 16..o + 32];
        let sig_bytes = &src[o + 33..o + 49];
        let sig_key = &src[o + 49..o + 65];
        let sig_check = xor16(&aes_ecb_encrypt16(UNITYCN_KEY, sig_key), sig_bytes);
        if &sig_check != UNITYCN_SIG {
            return Err("Invalid bundle encryption signature".into());
        }

        let cipher = UnityCnCipher::new(info_bytes, info_key);
        let blocks_info_off = ((o - base_off) + 65 + 15) & !15;
        let header_prefix = src[base_off..base_off + blocks_info_off].to_vec();
        let blocks_info = lz4_decompress(&src[base_off + blocks_info_off..base_off + blocks_info_off + c_info], u_info)?;
        let data_start = (blocks_info_off + c_info + 15) & !15;

        let mut data_hash = [0u8; 16];
        data_hash.copy_from_slice(&blocks_info[..16]);
        let mut bp = 16usize;
        let b_count = read_u32_be(&blocks_info, bp) as usize;
        bp += 4;
        let mut blocks = Vec::with_capacity(b_count);
        let mut cur_off = data_start;
        for _ in 0..b_count {
            let u_size = read_u32_be(&blocks_info, bp) as usize;
            let c_size = read_u32_be(&blocks_info, bp + 4) as usize;
            let flags = read_u16_be(&blocks_info, bp + 8);
            blocks.push(UnityCnBlock {
                u_size,
                c_size,
                flags,
                file_offset: cur_off,
            });
            cur_off += c_size;
            bp += 10;
        }
        let d_count = read_u32_be(&blocks_info, bp) as usize;
        bp += 4;
        let mut dirs = Vec::with_capacity(d_count);
        for _ in 0..d_count {
            let offset = read_i64_be(&blocks_info, bp);
            bp += 8;
            let size = read_i64_be(&blocks_info, bp);
            bp += 8;
            let flags = read_u32_be(&blocks_info, bp);
            bp += 4;
            let end = blocks_info[bp..].iter().position(|&b| b == 0).unwrap_or(0);
            let name = String::from_utf8_lossy(&blocks_info[bp..bp + end]).into_owned();
            bp += end + 1;
            dirs.push(UnityCnDir { offset, size, flags, name });
        }

        Ok(Self {
            src,
            base_off,
            fsize_off,
            cinfo_off,
            blocks_info_off,
            data_start,
            header_prefix,
            data_hash,
            blocks,
            dirs,
            cipher,
        })
    }

    pub fn read_block(&self, idx: usize) -> Result<Vec<u8>, String> {
        let blk = &self.blocks[idx];
        let mut cbuf = self.src[self.base_off + blk.file_offset..self.base_off + blk.file_offset + blk.c_size].to_vec();
        if (blk.flags & 0x100) != 0 {
            self.cipher.decrypt_block_in_place(&mut cbuf, idx);
        }
        let ct = blk.flags & 0x3F;
        if ct == 0 {
            Ok(cbuf)
        } else {
            lz4_decompress(&cbuf, blk.u_size)
        }
    }

    pub fn read_all(&self) -> Result<Vec<u8>, String> {
        let total: usize = self.blocks.iter().map(|b| b.u_size).sum();
        let mut out = Vec::with_capacity(total);
        for i in 0..self.blocks.len() {
            out.extend_from_slice(&self.read_block(i)?);
        }
        Ok(out)
    }
}
