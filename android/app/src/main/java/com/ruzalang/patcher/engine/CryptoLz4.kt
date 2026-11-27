package com.ruzalang.patcher.engine

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.FileInputStream
import java.security.MessageDigest
import java.util.zip.GZIPInputStream
import javax.crypto.Cipher
import javax.crypto.spec.SecretKeySpec

object CryptoLz4 {
    val UNITYCN_KEY: ByteArray = "z4l23bh5#35h&321".toByteArray(Charsets.US_ASCII)
    val UNITYCN_SIG: ByteArray = "#\$manjuuunity*!@".toByteArray(Charsets.US_ASCII)
    private val HEX_CHARS = "0123456789ABCDEF".toCharArray()

    fun gunzip(data: ByteArray): ByteArray {
        GZIPInputStream(ByteArrayInputStream(data)).use { gz ->
            val out = ByteArrayOutputStream(data.size * 3)
            val buf = ByteArray(65536)
            while (true) {
                val n = gz.read(buf)
                if (n <= 0) break
                out.write(buf, 0, n)
            }
            return out.toByteArray()
        }
    }

    fun hexUpper(bytes: ByteArray): String {
        val out = CharArray(bytes.size * 2)
        for (i in bytes.indices) {
            val v = bytes[i].toInt() and 0xFF
            out[i * 2] = HEX_CHARS[v ushr 4]
            out[i * 2 + 1] = HEX_CHARS[v and 0x0F]
        }
        return String(out)
    }

    fun sha256Bytes(data: ByteArray): String {
        val md = MessageDigest.getInstance("SHA-256")
        md.update(data)
        return hexUpper(md.digest())
    }

    fun sha256File(file: File): String {
        val md = MessageDigest.getInstance("SHA-256")
        val buf = ByteArray(4 * 1024 * 1024)
        FileInputStream(file).use { fis ->
            while (true) {
                val n = fis.read(buf)
                if (n <= 0) break
                md.update(buf, 0, n)
            }
        }
        return hexUpper(md.digest())
    }

    fun fnv1a64(buf: ByteArray, start: Int, len: Int): Long {
        var h = (0xcbf29ce4L shl 32) or 0x84222325L
        val prime = 0x100000001b3L
        val end = start + len
        for (i in start until end) {
            h = h xor (buf[i].toLong() and 0xFFL)
            h *= prime
        }
        return h
    }

    fun readI64Le(b: ByteArray, o: Int): Long {
        var v = 0L
        for (i in 0 until 8) {
            v = v or ((b[o + i].toLong() and 0xFFL) shl (i * 8))
        }
        return v
    }

    fun findBytes(haystack: ByteArray, needle: ByteArray, start: Int = 0): Int {
        if (needle.isEmpty() || start < 0 || start + needle.size > haystack.size) return -1
        val limit = haystack.size - needle.size
        val first = needle[0]
        var i = start
        while (i <= limit) {
            if (haystack[i] == first) {
                var ok = true
                for (j in 1 until needle.size) {
                    if (haystack[i + j] != needle[j]) {
                        ok = false
                        break
                    }
                }
                if (ok) return i
            }
            i++
        }
        return -1
    }

    fun findStaticUiSubBundle(buf: ByteArray): Triple<Int, Int, Boolean>? {
        val magic = byteArrayOf('U'.code.toByte(), 'n'.code.toByte(), 'i'.code.toByte(), 't'.code.toByte(), 'y'.code.toByte(), 'F'.code.toByte(), 'S'.code.toByte(), 0)
        val cabName = "CAB-d373fa6fedc5b221683a433ac01d8eb0"
        val marker = "ast_staticTextKeyConfig".toByteArray(Charsets.US_ASCII)
        if (buf.size < 128) return null
        val fallbackOffsets = ArrayList<Int>(2)
        var pos = 0
        while (pos + 8 <= buf.size) {
            val off = findBytes(buf, magic, pos)
            if (off < 0) break
            pos = off + 8
            try {
                val bundle = UnityCnBundle.parse(buf, off)
                if (bundle.dirs.size == 1 && bundle.dirs[0].name == cabName) {
                    return Triple(off, bundle.fsize, true)
                }
                if (bundle.dirs.size == 1 &&
                    bundle.fsize in 30_000..500_000 &&
                    bundle.dirs[0].size in 80_000L..450_000L
                ) {
                    fallbackOffsets.add(off)
                }
            } catch (_: Throwable) {
            }
        }
        for (off in fallbackOffsets) {
            try {
                val bundle = UnityCnBundle.parse(buf, off)
                val sf = bundle.readAll()
                if (findBytes(sf, marker) >= 0) {
                    return Triple(off, bundle.fsize, false)
                }
            } catch (_: Throwable) {
            }
        }
        return null
    }

    fun xorMMapRangeInPlace(buf: ByteArray, startIdx: Int, endIdx: Int, fileStartOffset: Int, key4096: ByteArray) {
        var pos = fileStartOffset
        for (i in startIdx until endIdx) {
            val shift = key4096[(pos ushr 12) and 0xFF].toInt() and 0xFF
            buf[i] = (buf[i].toInt() xor key4096[((pos and 0xFFF) + shift) and 0xFFF].toInt()).toByte()
            pos++
        }
    }

    fun readU16Le(b: ByteArray, o: Int): Int =
        (b[o].toInt() and 0xFF) or ((b[o + 1].toInt() and 0xFF) shl 8)

    fun readI32Le(b: ByteArray, o: Int): Int =
        (b[o].toInt() and 0xFF) or
            ((b[o + 1].toInt() and 0xFF) shl 8) or
            ((b[o + 2].toInt() and 0xFF) shl 16) or
            ((b[o + 3].toInt() and 0xFF) shl 24)

    fun writeI32Le(b: ByteArray, o: Int, v: Int) {
        b[o] = (v and 0xFF).toByte()
        b[o + 1] = ((v ushr 8) and 0xFF).toByte()
        b[o + 2] = ((v ushr 16) and 0xFF).toByte()
        b[o + 3] = ((v ushr 24) and 0xFF).toByte()
    }

    fun readU16Be(b: ByteArray, o: Int): Int =
        ((b[o].toInt() and 0xFF) shl 8) or (b[o + 1].toInt() and 0xFF)

    fun readI32Be(b: ByteArray, o: Int): Int =
        ((b[o].toInt() and 0xFF) shl 24) or
            ((b[o + 1].toInt() and 0xFF) shl 16) or
            ((b[o + 2].toInt() and 0xFF) shl 8) or
            (b[o + 3].toInt() and 0xFF)

    fun readI64Be(b: ByteArray, o: Int): Long {
        var v = 0L
        for (i in 0 until 8) {
            v = (v shl 8) or (b[o + i].toLong() and 0xFFL)
        }
        return v
    }

    fun writeI32Be(b: ByteArray, o: Int, v: Int) {
        b[o] = ((v ushr 24) and 0xFF).toByte()
        b[o + 1] = ((v ushr 16) and 0xFF).toByte()
        b[o + 2] = ((v ushr 8) and 0xFF).toByte()
        b[o + 3] = (v and 0xFF).toByte()
    }

    fun writeI64Be(b: ByteArray, o: Int, v: Long) {
        for (i in 0 until 8) {
            b[o + i] = ((v ushr (56 - i * 8)) and 0xFFL).toByte()
        }
    }

    fun aesEcbEncrypt16(key: ByteArray, block: ByteArray, blockOff: Int = 0): ByteArray {
        val cipher = Cipher.getInstance("AES/ECB/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, SecretKeySpec(key, "AES"))
        return cipher.doFinal(block, blockOff, 16)
    }

    fun xor16(a: ByteArray, aOff: Int, b: ByteArray, bOff: Int): ByteArray {
        val out = ByteArray(16)
        for (i in 0 until 16) {
            out[i] = (a[aOff + i].toInt() xor b[bOff + i].toInt()).toByte()
        }
        return out
    }

    fun lz4Decompress(src: ByteArray, dstLen: Int): ByteArray {
        val dst = ByteArray(dstLen)
        var sp = 0
        var dp = 0
        while (sp < src.size) {
            val token = src[sp++].toInt() and 0xFF
            var litLen = token ushr 4
            if (litLen == 15) {
                while (true) {
                    if (sp >= src.size) error("LZ4 truncated literal length")
                    val b = src[sp++].toInt() and 0xFF
                    litLen += b
                    if (b != 255) break
                }
            }
            if (sp + litLen > src.size || dp + litLen > dstLen) error("LZ4 literal overflow")
            System.arraycopy(src, sp, dst, dp, litLen)
            sp += litLen
            dp += litLen
            if (sp >= src.size) break
            if (sp + 2 > src.size) error("LZ4 truncated offset")
            val offset = (src[sp].toInt() and 0xFF) or ((src[sp + 1].toInt() and 0xFF) shl 8)
            sp += 2
            if (offset == 0 || offset > dp) error("LZ4 invalid match offset")
            var matchLen = (token and 0x0F) + 4
            if ((token and 0x0F) == 15) {
                while (true) {
                    if (sp >= src.size) error("LZ4 truncated match length")
                    val b = src[sp++].toInt() and 0xFF
                    matchLen += b
                    if (b != 255) break
                }
            }
            if (dp + matchLen > dstLen) error("LZ4 match overflow")
            for (i in 0 until matchLen) {
                dst[dp + i] = dst[dp - offset + i]
            }
            dp += matchLen
        }
        if (dp != dstLen) error("LZ4 decompress size mismatch: $dp != $dstLen")
        return dst
    }

    private class Lz4Seq(
        var litStart: Int,
        var litLen: Int,
        var offset: Int,
        var matchLen: Int,
    ) {
        fun copy() = Lz4Seq(litStart, litLen, offset, matchLen)
    }

    private fun insertHash(src: ByteArray, pos: Int, head: IntArray, prev: IntArray) {
        val v = readI32Le(src, pos)
        val h = (v * -1640531535) ushr 16 // 2654435761u32
        prev[pos] = head[h]
        head[h] = pos
    }

    private fun findBestMatch(
        src: ByteArray,
        pos: Int,
        matchlimit: Int,
        head: IntArray,
        prev: IntArray,
        chainDepth: Int,
    ): Long {
        val v = readI32Le(src, pos)
        val h = (v * -1640531535) ushr 16
        var r = head[h]
        var bestLen = 3
        var bestOff = 0
        var steps = 0
        while (r >= 0 && steps < chainDepth) {
            steps++
            val dist = pos - r
            if (dist > 65535) break
            if (src[r + bestLen] == src[pos + bestLen] && readI32Le(src, r) == v) {
                var ml = 4
                while (pos + ml < matchlimit && src[r + ml] == src[pos + ml]) {
                    ml++
                }
                if (ml > bestLen) {
                    bestLen = ml
                    bestOff = dist
                    if (pos + ml >= matchlimit) break
                }
            }
            r = prev[r]
        }
        return (bestLen.toLong() shl 32) or (bestOff.toLong() and 0xFFFFFFFFL)
    }

    private fun findHcSequences(src: ByteArray, chainDepth: Int): Pair<ArrayList<Lz4Seq>, Int> {
        val n = src.size
        if (n < 13) return Pair(ArrayList(), 0)
        val head = IntArray(1 shl 16) { -1 }
        val prev = IntArray(n) { -1 }
        val mflimit = n - 12
        val matchlimit = n - 5
        val seqs = ArrayList<Lz4Seq>(n / 16 + 16)
        var anchor = 0
        var ip = 0
        while (ip < mflimit) {
            val res = findBestMatch(src, ip, matchlimit, head, prev, chainDepth)
            val bestLen = (res ushr 32).toInt()
            val bestOff = res.toInt()
            insertHash(src, ip, head, prev)
            if (bestLen >= 4) {
                if (ip + 1 < mflimit) {
                    val b2Len = (findBestMatch(src, ip + 1, matchlimit, head, prev, chainDepth) ushr 32).toInt()
                    if (b2Len > bestLen) {
                        ip++
                        continue
                    }
                }
                seqs.add(Lz4Seq(anchor, ip - anchor, bestOff, bestLen))
                for (k in 1 until bestLen) {
                    if (ip + k < mflimit) insertHash(src, ip + k, head, prev)
                }
                ip += bestLen
                anchor = ip
            } else {
                ip++
            }
        }
        return Pair(seqs, anchor)
    }

    private fun measureEncodedSize(seqs: List<Lz4Seq>, count: Int, tailStart: Int, totalLen: Int): Int {
        var sz = 0
        for (i in 0 until count) {
            val s = seqs[i]
            sz += 1 + s.litLen + 2
            if (s.litLen >= 15) sz += ((s.litLen - 15) / 255) + 1
            if (s.matchLen - 4 >= 15) sz += ((s.matchLen - 19) / 255) + 1
        }
        val tailLen = totalLen - tailStart
        sz += 1 + tailLen
        if (tailLen >= 15) sz += ((tailLen - 15) / 255) + 1
        return sz
    }

    private fun encodeSequences(src: ByteArray, seqs: List<Lz4Seq>, tailStart: Int): ByteArray {
        val total = measureEncodedSize(seqs, seqs.size, tailStart, src.size)
        val out = ByteArray(total)
        var wp = 0
        for (s in seqs) {
            val tl = minOf(s.litLen, 15)
            val tm = minOf(s.matchLen - 4, 15)
            out[wp++] = ((tl shl 4) or tm).toByte()
            if (s.litLen >= 15) {
                var rem = s.litLen - 15
                while (rem >= 255) {
                    out[wp++] = 255.toByte()
                    rem -= 255
                }
                out[wp++] = rem.toByte()
            }
            System.arraycopy(src, s.litStart, out, wp, s.litLen)
            wp += s.litLen
            out[wp++] = (s.offset and 0xFF).toByte()
            out[wp++] = ((s.offset ushr 8) and 0xFF).toByte()
            if (s.matchLen - 4 >= 15) {
                var rem = s.matchLen - 19
                while (rem >= 255) {
                    out[wp++] = 255.toByte()
                    rem -= 255
                }
                out[wp++] = rem.toByte()
            }
        }
        val tailLen = src.size - tailStart
        val ttl = minOf(tailLen, 15)
        out[wp++] = (ttl shl 4).toByte()
        if (tailLen >= 15) {
            var rem = tailLen - 15
            while (rem >= 255) {
                out[wp++] = 255.toByte()
                rem -= 255
            }
            out[wp++] = rem.toByte()
        }
        System.arraycopy(src, tailStart, out, wp, tailLen)
        wp += tailLen
        if (wp != total) error("LZ4 encode length mismatch ($wp != $total)")
        return out
    }

    fun lz4Compress(src: ByteArray, chainDepth: Int = 1024): ByteArray {
        val (seqs, anchor) = findHcSequences(src, chainDepth)
        return encodeSequences(src, seqs, anchor)
    }

    fun lz4CompressExact(src: ByteArray, targetSize: Int, chainDepth: Int = 1024): ByteArray {
        val (allSeqs, anchor) = findHcSequences(src, chainDepth)
        val minSize = measureEncodedSize(allSeqs, allSeqs.size, anchor, src.size)
        if (minSize == targetSize) return encodeSequences(src, allSeqs, anchor)
        if (minSize > targetSize) error("Cannot fit LZ4HC block into targetSize=$targetSize (min=$minSize)")
        val rawSize = measureEncodedSize(emptyList(), 0, 0, src.size)
        if (rawSize < targetSize) error("Uncompressed LZ4 literal size ($rawSize) < targetSize=$targetSize")

        var lo = 0
        var hi = allSeqs.size
        while (lo < hi) {
            val mid = (lo + hi) ushr 1
            val tStart = if (mid == 0) 0 else {
                val s = allSeqs[mid - 1]
                s.litStart + s.litLen + s.matchLen
            }
            val len = measureEncodedSize(allSeqs, mid, tStart, src.size)
            if (len <= targetSize) hi = mid else lo = mid + 1
        }

        val seqs = ArrayList<Lz4Seq>(lo)
        for (i in 0 until lo) seqs.add(allSeqs[i].copy())
        var tailStart = if (lo == 0) 0 else {
            val s = seqs[lo - 1]
            s.litStart + s.litLen + s.matchLen
        }
        var curSize = measureEncodedSize(seqs, seqs.size, tailStart, src.size)

        if (seqs.isNotEmpty()) {
            val last = seqs[seqs.size - 1]
            while (last.matchLen > 4 && curSize < targetSize) {
                last.matchLen--
                val nextSize = measureEncodedSize(seqs, seqs.size, tailStart - 1, src.size)
                if (nextSize <= targetSize) {
                    tailStart--
                    curSize = nextSize
                } else {
                    last.matchLen++
                    break
                }
            }
        }

        if (seqs.size >= 2) {
            for (j in seqs.size - 2 downTo 0) {
                if (curSize >= targetSize) break
                val sj = seqs[j]
                val sNext = seqs[j + 1]
                while (sj.matchLen > 4 && curSize < targetSize) {
                    sj.matchLen--
                    sNext.litStart--
                    sNext.litLen++
                    val nextSize = measureEncodedSize(seqs, seqs.size, tailStart, src.size)
                    if (nextSize <= targetSize) {
                        curSize = nextSize
                    } else {
                        sj.matchLen++
                        sNext.litStart++
                        sNext.litLen--
                        break
                    }
                }
            }
        }

        if (curSize != targetSize) error("Failed to tune LZ4HC block to exact size $targetSize (got $curSize)")
        val buf = encodeSequences(src, seqs, tailStart)
        val verify = lz4Decompress(buf, src.size)
        if (!verify.contentEquals(src)) error("Exact LZ4HC round-trip mismatch")
        return buf
    }
}

class UnityCnCipher(infoBytes: ByteArray, infoBytesOff: Int, infoKey: ByteArray, infoKeyOff: Int) {
    private val index = IntArray(16)
    private val invIndex = IntArray(16)
    private val s = IntArray(16)

    init {
        val enc = CryptoLz4.aesEcbEncrypt16(CryptoLz4.UNITYCN_KEY, infoKey, infoKeyOff)
        val dec16 = CryptoLz4.xor16(enc, 0, infoBytes, infoBytesOff)
        for (i in 0 until 8) {
            val b = dec16[i].toInt() and 0xFF
            index[2 * i] = b ushr 4
            index[2 * i + 1] = b and 0x0F
        }
        for (i in 0 until 16) {
            invIndex[index[i]] = i
        }
        for (i in 0 until 4) {
            val b0 = dec16[8 + 2 * i].toInt() and 0xFF
            val b1 = dec16[8 + 2 * i + 1].toInt() and 0xFF
            s[i] = b0 ushr 4
            s[4 + i] = b0 and 0x0F
            s[8 + i] = b1 ushr 4
            s[12 + i] = b1 and 0x0F
        }
    }

    private fun r9At(idx: Int): Int =
        (s[12 + ((idx ushr 6) and 3)] +
            s[8 + ((idx ushr 4) and 3)] +
            s[4 + ((idx ushr 2) and 3)] +
            s[idx and 3]) and 0xFF

    private fun decryptByte(buf: ByteArray, off: Int, idx: Int): Int {
        val b = buf[off].toInt() and 0xFF
        val r9 = r9At(idx)
        val hi = (index[b ushr 4] - r9) and 0x0F
        val lo = (index[b and 0x0F] - r9) and 0x0F
        val res = (hi shl 4) or lo
        buf[off] = res.toByte()
        return res
    }

    private fun encryptByte(buf: ByteArray, off: Int, idx: Int): Int {
        val p = buf[off].toInt() and 0xFF
        val r9 = r9At(idx)
        val hi = invIndex[((p ushr 4) + r9) and 0x0F]
        val lo = invIndex[((p and 0x0F) + r9) and 0x0F]
        buf[off] = ((hi shl 4) or lo).toByte()
        return p
    }

    fun decryptBlockInPlace(buf: ByteArray, blockIndex: Int) {
        var p = 0
        var seqIdx = blockIndex
        while (p < buf.size) {
            var bIdx = seqIdx++
            val tok = decryptByte(buf, p++, bIdx++)
            var litLen = tok ushr 4
            if (litLen == 15) {
                while (true) {
                    val e = decryptByte(buf, p++, bIdx++)
                    litLen += e
                    if (e != 255) break
                }
            }
            p += litLen
            if (p >= buf.size) break
            decryptByte(buf, p++, bIdx++)
            decryptByte(buf, p++, bIdx++)
            if ((tok and 0x0F) == 15) {
                while (true) {
                    val e = decryptByte(buf, p++, bIdx++)
                    if (e != 255) break
                }
            }
        }
    }

    fun encryptBlockInPlace(buf: ByteArray, blockIndex: Int) {
        var p = 0
        var seqIdx = blockIndex
        while (p < buf.size) {
            var bIdx = seqIdx++
            val tok = encryptByte(buf, p++, bIdx++)
            var litLen = tok ushr 4
            if (litLen == 15) {
                while (true) {
                    val e = encryptByte(buf, p++, bIdx++)
                    litLen += e
                    if (e != 255) break
                }
            }
            p += litLen
            if (p >= buf.size) break
            encryptByte(buf, p++, bIdx++)
            encryptByte(buf, p++, bIdx++)
            if ((tok and 0x0F) == 15) {
                while (true) {
                    val e = encryptByte(buf, p++, bIdx++)
                    if (e != 255) break
                }
            }
        }
    }
}

data class UnityCnBlock(val uSize: Int, val cSize: Int, val flags: Int, val fileOffset: Int)
data class UnityCnDir(val offset: Long, val size: Long, val flags: Int, val name: String)

class UnityCnBundle(
    val src: ByteArray,
    val baseOff: Int,
    val fsize: Int,
    val fsizeOff: Int,
    val cinfoOff: Int,
    val blocksInfoOff: Int,
    val dataStart: Int,
    val headerPrefix: ByteArray,
    val dataHash: ByteArray,
    val blocks: List<UnityCnBlock>,
    val dirs: List<UnityCnDir>,
    val cipher: UnityCnCipher,
) {
    fun readBlock(idx: Int): ByteArray {
        val blk = blocks[idx]
        val cbuf = src.copyOfRange(baseOff + blk.fileOffset, baseOff + blk.fileOffset + blk.cSize)
        if ((blk.flags and 0x100) != 0) {
            cipher.decryptBlockInPlace(cbuf, idx)
        }
        val ct = blk.flags and 0x3F
        return if (ct == 0) cbuf else CryptoLz4.lz4Decompress(cbuf, blk.uSize)
    }

    fun readAll(): ByteArray {
        val total = blocks.sumOf { it.uSize }
        val out = ByteArray(total)
        var wp = 0
        for (i in blocks.indices) {
            val part = readBlock(i)
            System.arraycopy(part, 0, out, wp, part.size)
            wp += part.size
        }
        return out
    }

    companion object {
        fun parse(src: ByteArray, baseOff: Int): UnityCnBundle {
            if (baseOff < 0 || baseOff + 64 > src.size) error("UnityFS header truncated")
            var o = baseOff
            fun readCStr(): String {
                val maxEnd = minOf(o + 128, src.size)
                var end = o
                while (end < maxEnd && src[end] != 0.toByte()) end++
                if (end >= maxEnd) error("Missing null terminator in UnityFS header")
                val s = String(src, o, end - o, Charsets.UTF_8)
                o = end + 1
                return s
            }
            val sig = readCStr()
            if (sig != "UnityFS") error("Expected UnityFS, got $sig")
            if (o + 4 > src.size) error("UnityFS version truncated")
            val ver = CryptoLz4.readI32Be(src, o); o += 4
            readCStr() // pver
            readCStr() // ever
            if (o + 20 > src.size) error("UnityFS size/info header truncated")
            val fsizeOff = o - baseOff
            val fsizeLong = CryptoLz4.readI64Be(src, o)
            if (fsizeLong <= 0L || fsizeLong > (src.size - baseOff).toLong()) {
                error("UnityFS fsize out of range: $fsizeLong")
            }
            val fsize = fsizeLong.toInt()
            o += 8
            val cinfoOff = o - baseOff
            val cInfo = CryptoLz4.readI32Be(src, o); o += 4
            val uInfo = CryptoLz4.readI32Be(src, o); o += 4
            o += 4 // flags
            if (ver >= 7) {
                o = baseOff + (((o - baseOff) + 15) and 15.inv())
            }
            if (o + 65 > src.size) error("UnityCN key block truncated")
            val infoBytesOff = o
            val infoKeyOff = o + 16
            val sigBytesOff = o + 33
            val sigKeyOff = o + 49
            val sigCheck = CryptoLz4.xor16(
                CryptoLz4.aesEcbEncrypt16(CryptoLz4.UNITYCN_KEY, src, sigKeyOff),
                0,
                src,
                sigBytesOff,
            )
            if (!sigCheck.contentEquals(CryptoLz4.UNITYCN_SIG)) {
                error("Invalid bundle encryption signature")
            }
            val cipher = UnityCnCipher(src, infoBytesOff, src, infoKeyOff)
            val blocksInfoOff = ((o - baseOff) + 65 + 15) and 15.inv()
            if (cInfo < 0 || uInfo < 0 || uInfo > 8 * 1024 * 1024 || baseOff + blocksInfoOff + cInfo > src.size) {
                error("UnityFS blocksInfo out of bounds")
            }
            val headerPrefix = src.copyOfRange(baseOff, baseOff + blocksInfoOff)
            val blocksInfoComp = src.copyOfRange(baseOff + blocksInfoOff, baseOff + blocksInfoOff + cInfo)
            val blocksInfo = CryptoLz4.lz4Decompress(blocksInfoComp, uInfo)
            val dataStart = (blocksInfoOff + cInfo + 15) and 15.inv()
            if (blocksInfo.size < 24) error("UnityFS blocksInfo too small")

            val dataHash = blocksInfo.copyOfRange(0, 16)
            var bp = 16
            val bCount = CryptoLz4.readI32Be(blocksInfo, bp); bp += 4
            if (bCount < 0 || bp + bCount * 10 + 4 > blocksInfo.size) error("UnityFS block table truncated")
            val blocks = ArrayList<UnityCnBlock>(bCount)
            var curOff = dataStart
            for (i in 0 until bCount) {
                val uSize = CryptoLz4.readI32Be(blocksInfo, bp)
                val cSize = CryptoLz4.readI32Be(blocksInfo, bp + 4)
                val flags = CryptoLz4.readU16Be(blocksInfo, bp + 8)
                blocks.add(UnityCnBlock(uSize, cSize, flags, curOff))
                curOff += cSize
                bp += 10
            }
            val dCount = CryptoLz4.readI32Be(blocksInfo, bp); bp += 4
            if (dCount < 0 || dCount > 10000) error("UnityFS dir count invalid")
            val dirs = ArrayList<UnityCnDir>(dCount)
            for (i in 0 until dCount) {
                if (bp + 20 > blocksInfo.size) error("UnityFS dir table truncated")
                val offset = CryptoLz4.readI64Be(blocksInfo, bp); bp += 8
                val size = CryptoLz4.readI64Be(blocksInfo, bp); bp += 8
                val flags = CryptoLz4.readI32Be(blocksInfo, bp); bp += 4
                var end = bp
                while (end < blocksInfo.size && blocksInfo[end] != 0.toByte()) end++
                val name = String(blocksInfo, bp, end - bp, Charsets.UTF_8)
                bp = end + 1
                dirs.add(UnityCnDir(offset, size, flags, name))
            }
            return UnityCnBundle(
                src = src,
                baseOff = baseOff,
                fsize = fsize,
                fsizeOff = fsizeOff,
                cinfoOff = cinfoOff,
                blocksInfoOff = blocksInfoOff,
                dataStart = dataStart,
                headerPrefix = headerPrefix,
                dataHash = dataHash,
                blocks = blocks,
                dirs = dirs,
                cipher = cipher,
            )
        }
    }
}
