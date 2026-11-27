package com.ruzalang.patcher.engine

import java.io.ByteArrayOutputStream
import java.io.File

enum class PatchLang(val shortCode: String, val label: String) {
    EN("EN", "English (EN)"),
    ID("ID", "Bahasa Indonesia (ID)");

    companion object {
        fun fromCode(code: String?): PatchLang =
            if (code.equals("id", ignoreCase = true)) ID else EN
    }
}

enum class LogLevel(val wire: String) {
    INFO("info"), OK("ok"), WARN("warn"), ERR("err"), DIFF("diff")
}

enum class CustomBase(val wire: String) {
    EN("en"),
    ID("id"),
    JP("jp");

    companion object {
        fun fromStr(raw: String?): CustomBase = when (raw?.trim()?.lowercase()) {
            "id" -> ID
            "jp", "none", "orig", "original" -> JP
            else -> EN
        }
    }
}

data class CustomPackMeta(
    val fileName: String,
    val name: String,
    val code: String,
    val author: String,
    val base: String,
    val vstringCount: Int,
    val staticUiCount: Int,
    val builtinCount: Int,
    val totalCount: Int,
)

data class CustomPack(
    val meta: CustomPackMeta,
    val base: CustomBase,
    val vstring: HashMap<Int, ByteArray>,
    val staticUi: HashMap<String, String>,
    val builtin: HashMap<String, String>,
)

/**
 * Tiny pure-Kotlin RFC 8259 JSON parser (zero external deps, works identically in JVM unit tests & Android).
 */
private class MiniJsonParser(private val src: String) {
    private var i = 0
    private val n = src.length

    private fun skipWs() {
        while (i < n && (src[i] == ' ' || src[i] == '\t' || src[i] == '\r' || src[i] == '\n')) i++
    }

    fun parseRootObject(): LinkedHashMap<String, Any?> {
        if (n > 0 && src[0] == '\uFEFF') i = 1
        skipWs()
        if (i >= n || src[i] != '{') error("Custom pack JSON root must be an object { ... }.")
        val obj = parseObject()
        skipWs()
        if (i != n) error("Unexpected trailing content in custom pack JSON at offset $i.")
        return obj
    }

    private fun parseValue(): Any? {
        skipWs()
        if (i >= n) error("Unexpected end of JSON.")
        return when (src[i]) {
            '{' -> parseObject()
            '[' -> parseArray()
            '"' -> parseString()
            't' -> { expectLiteral("true"); true }
            'f' -> { expectLiteral("false"); false }
            'n' -> { expectLiteral("null"); null }
            else -> parseNumber()
        }
    }

    private fun expectLiteral(lit: String) {
        if (!src.startsWith(lit, i)) error("Invalid JSON token at offset $i.")
        i += lit.length
    }

    private fun parseNumber(): String {
        val start = i
        if (i < n && src[i] == '-') i++
        while (i < n && (src[i] in '0'..'9' || src[i] == '.' || src[i] == 'e' || src[i] == 'E' || src[i] == '+' || src[i] == '-')) i++
        if (i == start) error("Invalid JSON token '${src.getOrNull(i)}' at offset $i.")
        return src.substring(start, i)
    }

    private fun parseArray(): ArrayList<Any?> {
        i++ // '['
        val list = ArrayList<Any?>()
        skipWs()
        if (i < n && src[i] == ']') { i++; return list }
        while (true) {
            list.add(parseValue())
            skipWs()
            if (i >= n) error("Unterminated JSON array.")
            when (src[i]) {
                ',' -> i++
                ']' -> { i++; return list }
                else -> error("Expected ',' or ']' in JSON array at offset $i.")
            }
        }
    }

    private fun parseObject(): LinkedHashMap<String, Any?> {
        i++ // '{'
        val map = LinkedHashMap<String, Any?>()
        skipWs()
        if (i < n && src[i] == '}') { i++; return map }
        while (true) {
            skipWs()
            if (i >= n || src[i] != '"') error("Expected object key string at offset $i.")
            val k = parseString()
            skipWs()
            if (i >= n || src[i] != ':') error("Expected ':' after key \"$k\" at offset $i.")
            i++
            val v = parseValue()
            map[k] = v
            skipWs()
            if (i >= n) error("Unterminated JSON object.")
            when (src[i]) {
                ',' -> i++
                '}' -> { i++; return map }
                else -> error("Expected ',' or '}' in JSON object at offset $i.")
            }
        }
    }

    private fun parseString(): String {
        i++ // opening '"'
        val sb = StringBuilder()
        while (i < n) {
            val ch = src[i++]
            when (ch) {
                '"' -> return sb.toString()
                '\\' -> {
                    if (i >= n) error("Unterminated string escape.")
                    when (val esc = src[i++]) {
                        '"' -> sb.append('"')
                        '\\' -> sb.append('\\')
                        '/' -> sb.append('/')
                        'b' -> sb.append('\b')
                        'f' -> sb.append('\u000C')
                        'n' -> sb.append('\n')
                        'r' -> sb.append('\r')
                        't' -> sb.append('\t')
                        'u' -> {
                            if (i + 4 > n) error("Incomplete \\uXXXX escape at offset $i.")
                            val hex = src.substring(i, i + 4)
                            val code = hex.toIntOrNull(16) ?: error("Invalid \\u$hex escape.")
                            sb.append(code.toChar())
                            i += 4
                        }
                        else -> error("Invalid escape \\$esc at offset $i.")
                    }
                }
                else -> sb.append(ch)
            }
        }
        error("Unterminated JSON string.")
    }
}

interface PayloadLoader {
    fun load(name: String): ByteArray
}

object PatchEngine {
    const val ANDROID_PACKAGE_ID = "com.Yostar.AzurpromiliaJP"
    const val ANDROID_EXPECTED_VERSION = "1.0.0"
    const val ANDROID_EXPECTED_RES_REV = "3219596"
    const val ANDROID_TOTAL_STRINGS = 74055 + 3135 // 77,190
    const val ANDROID_STATIC_UI_SUB_OFFSET = 4200075

    const val REMOTE_FILES_ROOT = "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files"
    const val REMOTE_VSTRING =
        "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files/.res/raw_base_package/BundleFiles/1qnamndresrohzjhuvt2lw.r"
    const val REMOTE_STATIC_UI =
        "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files/.res/default_package/BundleFiles/nibdvaxcnfh3en5bnvhtw.r"
    const val REMOTE_RAW_MANIFEST_DIR =
        "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files/.res/raw_base_package/ManifestFiles"
    const val REMOTE_DEFAULT_MANIFEST_DIR =
        "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files/.res/default_package/ManifestFiles"

    // Verified SHA-256 constants (v0.8.0 — 100% bit-identical with desktop PC/ADB engine)
    const val ANDROID_VSTRING_ORIG_SHA = "C6BA69F2403102A9D4DBF2B3AB079ABBE7DE25896ABDB3969DD5B7988A2DB013"
    const val ANDROID_STATIC_UI_ORIG_SHA = "53A36C59B4ABABF9457ED37B195092DCAFD491C2D95D9BA75A61998C0138D22A"
    const val ANDROID_VSTRING_EN_SHA = "0739151C287863A4CD45150CC85DE3DDFDD3A7587FB159437F3F5FEED513A4E5"
    const val ANDROID_STATIC_UI_EN_SHA = "2A4428AE21009DB19621A123B1A2561ED1F83A685B4C9160B94BA74CD50E8DAC"
    const val ANDROID_VSTRING_ID_SHA = "BD9A20B6BE6F9228E12F435F72374386F8C15D270E30B442927401A2EB36A0AF"
    const val ANDROID_STATIC_UI_ID_SHA = "0017773EB20E5B56D3E02179A9F94A6FBC862B48CFD9FF7CF443CCAE9E6A64B2"

    private fun isStaticUiKey(k: String): Boolean {
        val parts = k.split('+')
        if (parts.size > 2) return false
        val hex = parts[0]
        if (hex.length != 8 || !hex.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }) return false
        return parts.size == 1 || (parts[1].isNotEmpty() && parts[1].all { it in '0'..'9' })
    }

    private fun sanitizePackCode(raw: String): String {
        val cleaned = raw.trim().filter { it.isLetterOrDigit() || it == '-' || it == '_' }.take(8).uppercase()
        return cleaned.ifEmpty { "CUSTOM" }
    }

    @Suppress("UNCHECKED_CAST")
    fun parseCustomPackJson(raw: String, fileName: String = "custom-pack.json"): CustomPack {
        val root = MiniJsonParser(raw).parseRootObject()
        val defaultName = fileName.substringBeforeLast('.').ifBlank { "Custom Pack" }
        val metaObj = root["meta"] as? Map<String, Any?>
        val name = (metaObj?.get("name") as? String)?.trim()?.ifEmpty { null } ?: defaultName
        val code = sanitizePackCode((metaObj?.get("code") as? String) ?: "CUSTOM")
        val author = (metaObj?.get("author") as? String)?.trim().orEmpty()
        val base = CustomBase.fromStr(metaObj?.get("base") as? String)

        val vstring = HashMap<Int, ByteArray>()
        val staticUi = HashMap<String, String>()
        val builtin = HashMap<String, String>()

        val vsObj = root["vstring"] as? Map<String, Any?>
        val uiObj = root["static_ui"] as? Map<String, Any?>
        val biObj = root["builtin"] as? Map<String, Any?>
        val hasStructured = vsObj != null || uiObj != null || biObj != null

        if (hasStructured) {
            vsObj?.forEach { (k, v) ->
                if (k.startsWith("_")) return@forEach
                val s = v as? String ?: error("Custom pack vstring[\"$k\"] must be a string.")
                val id = k.toIntOrNull() ?: error("Custom pack vstring key \"$k\" must be a numeric ID.")
                vstring[id] = s.toByteArray(Charsets.UTF_8)
            }
            uiObj?.forEach { (k, v) ->
                if (k.startsWith("_")) return@forEach
                val s = v as? String ?: error("Custom pack static_ui[\"$k\"] must be a string.")
                staticUi[k] = s
            }
            biObj?.forEach { (k, v) ->
                if (k.startsWith("_")) return@forEach
                val s = v as? String ?: error("Custom pack builtin[\"$k\"] must be a string.")
                builtin[k] = s
            }
        } else {
            root.forEach { (k, v) ->
                if (k == "meta" || k.startsWith("_")) return@forEach
                val s = v as? String ?: return@forEach
                if (isStaticUiKey(k)) {
                    staticUi[k] = s
                } else if (k.isNotEmpty() && k.all { it in '0'..'9' }) {
                    k.toIntOrNull()?.let { id -> vstring[id] = s.toByteArray(Charsets.UTF_8) }
                } else {
                    builtin[k] = s
                }
            }
        }

        val total = vstring.size + staticUi.size + builtin.size
        if (total == 0) {
            error("Custom pack contains 0 translation entries. Add entries under \"vstring\", \"static_ui\", or \"builtin\" (or provide a flat ID->text JSON dictionary).")
        }

        val meta = CustomPackMeta(
            fileName = fileName,
            name = name,
            code = code,
            author = author,
            base = base.wire,
            vstringCount = vstring.size,
            staticUiCount = staticUi.size,
            builtinCount = builtin.size,
            totalCount = total,
        )
        return CustomPack(meta, base, vstring, staticUi, builtin)
    }

    private fun loadPcVStringMap(gz: ByteArray): HashMap<Int, ByteArray> {
        val raw = CryptoLz4.gunzip(gz)
        val count = CryptoLz4.readI32Le(raw, 0)
        val map = HashMap<Int, ByteArray>(count * 4 / 3 + 16)
        var p = 4
        for (i in 0 until count) {
            val id = CryptoLz4.readI32Le(raw, p)
            val len = CryptoLz4.readI32Le(raw, p + 4)
            p += 8
            map[id] = raw.copyOfRange(p, p + len)
            p += len
        }
        return map
    }

    fun loadAndroidVStringMap(
        loader: PayloadLoader,
        lang: PatchLang,
        custom: CustomPack? = null,
    ): HashMap<Int, ByteArray> {
        val pcMap = when {
            custom != null -> when (custom.base) {
                CustomBase.EN -> loadPcVStringMap(loader.load("vstring_en.bin.gz"))
                CustomBase.ID -> loadPcVStringMap(loader.load("vstring_id.bin.gz"))
                CustomBase.JP -> HashMap(custom.vstring.size * 4 / 3 + 16)
            }
            lang == PatchLang.ID -> loadPcVStringMap(loader.load("vstring_id.bin.gz"))
            else -> loadPcVStringMap(loader.load("vstring_en.bin.gz"))
        }
        if (custom != null) {
            for ((id, bytes) in custom.vstring) {
                pcMap[id] = bytes
            }
        }
        val rawRuns = CryptoLz4.gunzip(loader.load("vstring_android_map.bin.gz"))
        if (rawRuns.size < 4) error("Corrupt Android VStringPool remap table")
        val runCount = CryptoLz4.readI32Le(rawRuns, 0)
        if (rawRuns.size != 4 + runCount * 12) error("Android VStringPool remap table length mismatch")
        val anMap = HashMap<Int, ByteArray>(pcMap.size * 4 / 3 + 16)
        for (i in 0 until runCount) {
            val base = 4 + i * 12
            val pcStart = CryptoLz4.readI32Le(rawRuns, base)
            val anStart = CryptoLz4.readI32Le(rawRuns, base + 4)
            val len = CryptoLz4.readI32Le(rawRuns, base + 8)
            for (k in 0 until len) {
                val tr = pcMap[pcStart + k]
                if (tr != null) anMap[anStart + k] = tr
            }
        }
        if (custom == null && anMap.size != pcMap.size) {
            error("Android VStringPool remap incomplete (${anMap.size} != ${pcMap.size})")
        }
        return anMap
    }

    fun loadStringDict(gz: ByteArray): HashMap<String, String> {
        val raw = CryptoLz4.gunzip(gz)
        val count = CryptoLz4.readI32Le(raw, 0)
        val map = HashMap<String, String>(count * 4 / 3 + 16)
        var p = 4
        for (i in 0 until count) {
            val klen = CryptoLz4.readU16Le(raw, p)
            p += 2
            val k = String(raw, p, klen, Charsets.UTF_8)
            p += klen
            val vlen = CryptoLz4.readI32Le(raw, p)
            p += 4
            val v = String(raw, p, vlen, Charsets.UTF_8)
            p += vlen
            map[k] = v
        }
        return map
    }

    /**
     * Low-RAM in-place Android VStringPool exact-size repacker (~145 MB peak RAM for 131 MB bundle).
     * Mutates [buf] in-place and returns the rewritten string count (74,055).
     */
    fun patchAndroidVStringInPlace(
        buf: ByteArray,
        loader: PayloadLoader,
        lang: PatchLang,
        log: (String, LogLevel) -> Unit,
        custom: CustomPack? = null,
        onSub: (Int, String) -> Unit,
    ): Int {
        if (buf.size < 4144) error("VStringPool file too small (${buf.size})")
        val activeCode = custom?.meta?.code ?: lang.shortCode
        onSub(0, "Remapping VStringPool ($activeCode) entries...")
        val key4096 = loader.load("mmap_key_4096.bin")
        val translations = loadAndroidVStringMap(loader, lang, custom)
        log("  Remapped ${translations.size} $activeCode VStringPool translations.", LogLevel.INFO)

        onSub(0, "Decrypting VStringPool (130 MB MMap)...")
        val h1 = buf.copyOfRange(4120, 4144)
        CryptoLz4.xorMMapRangeInPlace(h1, 0, h1.size, 4120, key4096)
        val hashBytes = CryptoLz4.readI32Le(h1, 0)
        val hashStart = CryptoLz4.readI32Le(h1, 4)
        if (hashStart != 4120) error("Unexpected VStringHashWR startPos: $hashStart")

        val strSecOff = 4120 + hashBytes
        if (strSecOff + 20 > buf.size) error("VStringPool string section offset out of bounds")

        // Decrypt string section in-place (zero extra 130 MB copy!)
        CryptoLz4.xorMMapRangeInPlace(buf, strSecOff, buf.size, strSecOff, key4096)
        val count = CryptoLz4.readI32Le(buf, strSecOff + 16)
        log("  Decrypted VStringPool: $count total strings.", LogLevel.INFO)

        val spans = arrayOf(
            Triple("base_jp_1", 1473040, 1541595),
            Triple("base_jp_2", 1613715, 1616061),
            Triple("hotfix_jp", 1642789, count - 1),
        )

        val inplaceIds = ArrayList<Int>(4)
        for (id in translations.keys) {
            if (id < 0 || id >= count) error("VStringPool ID #$id out of range (count=$count)")
            val inSpan = spans.any { (_, s, e) -> id in s..e }
            if (!inSpan) inplaceIds.add(id)
        }

        var rewritten = 0
        val totalTr = translations.size
        val len4 = ByteArray(4)

        for ((name, start, end) in spans) {
            if (end >= count || start > end) error("Invalid VStringPool span $name [$start..$end]")
            val spanStartAbs = CryptoLz4.readI32Le(buf, strSecOff + 20 + start * 4)
            val spanEndAbs = if (end + 1 < count) {
                CryptoLz4.readI32Le(buf, strSecOff + 20 + (end + 1) * 4)
            } else {
                buf.size
            }
            val capacity = spanEndAbs - spanStartAbs
            val packed = ByteArrayOutputStream(capacity)
            val newOffsets = IntArray(end - start + 1)
            var cursor = spanStartAbs

            for (id in start..end) {
                newOffsets[id - start] = cursor
                val tr = translations[id]
                if (tr != null) {
                    rewritten++
                    if (rewritten % 2500 == 0) {
                        onSub(rewritten, "Rewriting VStringPool [$activeCode / $name]: $rewritten / $totalTr")
                    }
                    CryptoLz4.writeI32Le(len4, 0, tr.size)
                    packed.write(len4)
                    packed.write(tr)
                    cursor += 4 + tr.size
                } else {
                    val oAbs = CryptoLz4.readI32Le(buf, strSecOff + 20 + id * 4)
                    val oLen = CryptoLz4.readI32Le(buf, oAbs)
                    packed.write(buf, oAbs, 4 + oLen)
                    cursor += 4 + oLen
                }
            }
            val used = cursor - spanStartAbs
            if (used > capacity) error("Span $name overflowed by ${used - capacity} bytes")

            // Write updated offsets + repacked span bytes directly into buf
            for (i in newOffsets.indices) {
                CryptoLz4.writeI32Le(buf, strSecOff + 20 + (start + i) * 4, newOffsets[i])
            }
            val packedBytes = packed.toByteArray()
            System.arraycopy(packedBytes, 0, buf, spanStartAbs, used)
            if (used < capacity) {
                buf.fill(0, spanStartAbs + used, spanStartAbs + capacity)
            }
            log("  Repacked $name [#$start..#$end]: $used/$capacity B (free slack ${capacity - used} B)", LogLevel.DIFF)
        }

        for (id in inplaceIds) {
            val oAbs = CryptoLz4.readI32Le(buf, strSecOff + 20 + id * 4)
            val oLen = CryptoLz4.readI32Le(buf, oAbs)
            val payload = translations[id]!!
            if (payload.size > oLen) error("In-place ID $id overflow (${payload.size} > $oLen)")
            CryptoLz4.writeI32Le(buf, oAbs, payload.size)
            System.arraycopy(payload, 0, buf, oAbs + 4, payload.size)
            if (payload.size < oLen) {
                buf.fill(0, oAbs + 4 + payload.size, oAbs + 4 + oLen)
            }
            rewritten++
        }

        // Verify all 74,055 strings before re-encryption
        onSub(rewritten, "Re-encrypting & verifying 74,055 VStringPool strings...")
        for ((id, expected) in translations) {
            val a = CryptoLz4.readI32Le(buf, strSecOff + 20 + id * 4)
            val l = CryptoLz4.readI32Le(buf, a)
            if (l != expected.size) error("VStringPool verification length mismatch on #$id")
            for (k in 0 until l) {
                if (buf[a + 4 + k] != expected[k]) error("VStringPool verification mismatch on #$id")
            }
        }

        // Re-encrypt string section in-place
        CryptoLz4.xorMMapRangeInPlace(buf, strSecOff, buf.size, strSecOff, key4096)
        return rewritten
    }

    private fun align4(n: Int): Int = (n + 3) and 3.inv()

    fun patchAndroidStaticUiBytes(
        origBuf: ByteArray,
        loader: PayloadLoader,
        lang: PatchLang,
        custom: CustomPack? = null,
        onSub: (Int, String) -> Unit,
    ): Pair<ByteArray, Int> {
        val subOffset = ANDROID_STATIC_UI_SUB_OFFSET
        val subSize = 83151
        val dataOff = 3392
        val mbNameOff = 3420
        val obj0SizeOff = 3220
        val origMbSize = 161392

        if (subOffset + subSize > origBuf.size) {
            error("Static UI sub-bundle offset out of range ($subOffset + $subSize > ${origBuf.size})")
        }
        val activeCode = custom?.meta?.code ?: lang.shortCode
        val trMap = when {
            custom != null -> when (custom.base) {
                CustomBase.EN -> loadStringDict(loader.load("static_ui_en.bin.gz"))
                CustomBase.ID -> loadStringDict(loader.load("static_ui_id.bin.gz"))
                CustomBase.JP -> HashMap(custom.staticUi.size * 4 / 3 + 16)
            }
            lang == PatchLang.ID -> loadStringDict(loader.load("static_ui_id.bin.gz"))
            else -> loadStringDict(loader.load("static_ui_en.bin.gz"))
        }
        if (custom != null) {
            for ((k, v) in custom.staticUi) {
                trMap[k] = v
            }
        }

        onSub(0, "Decrypting UnityCN ast_staticTextKeyConfig.asset...")
        val bundle = UnityCnBundle.parse(origBuf, subOffset)
        val origSf = bundle.readAll()

        var cur = mbNameOff + 28
        val langLen = CryptoLz4.readI32Le(origSf, cur); cur += 4
        val langTag = String(origSf, cur, langLen, Charsets.UTF_8)
        cur = align4(cur + langLen)
        val pairCount = CryptoLz4.readI32Le(origSf, cur); cur += 4

        val newMb = ByteArrayOutputStream(origMbSize)
        newMb.write(origSf, dataOff, (mbNameOff + 28) - dataOff)
        val len4 = ByteArray(4)
        val zeros = ByteArray(4)

        fun writeStr(s: String) {
            val b = s.toByteArray(Charsets.UTF_8)
            CryptoLz4.writeI32Le(len4, 0, b.size)
            newMb.write(len4)
            newMb.write(b)
            val pad = align4(b.size) - b.size
            if (pad > 0) newMb.write(zeros, 0, pad)
        }

        writeStr(langTag)
        CryptoLz4.writeI32Le(len4, 0, pairCount)
        newMb.write(len4)

        var replaced = 0
        for (i in 0 until pairCount) {
            val kl = CryptoLz4.readI32Le(origSf, cur); cur += 4
            val k = String(origSf, cur, kl, Charsets.UTF_8)
            cur = align4(cur + kl)
            val vl = CryptoLz4.readI32Le(origSf, cur); cur += 4
            val origV = String(origSf, cur, vl, Charsets.UTF_8)
            cur = align4(cur + vl)

            val tr = trMap[k]
            val v = if (tr != null) {
                replaced++
                if (replaced % 400 == 0) {
                    onSub(replaced, "Rewriting Static UI KeyConfig [$activeCode]: $replaced / 3,135")
                }
                tr
            } else {
                origV
            }
            writeStr(k)
            writeStr(v)
        }

        val newMbBytes = newMb.toByteArray()
        if (newMbBytes.size > origMbSize) {
            error("Rebuilt Static UI exceeds slot (${newMbBytes.size} > $origMbSize)")
        }

        val newSf = origSf.copyOf()
        newSf.fill(0, dataOff, dataOff + origMbSize)
        System.arraycopy(newMbBytes, 0, newSf, dataOff, newMbBytes.size)
        CryptoLz4.writeI32Le(newSf, obj0SizeOff, newMbBytes.size)

        onSub(replaced, "Compressing (LZ4HC exact) & UnityCN encrypting Static UI...")
        val u0 = bundle.blocks[0].uSize
        val u1 = bundle.blocks[1].uSize
        val totalBudget = subSize - bundle.dataStart
        val c0 = CryptoLz4.lz4Compress(newSf.copyOfRange(0, u0), 1024)
        val c1 = CryptoLz4.lz4CompressExact(newSf.copyOfRange(u0, u0 + u1), totalBudget - c0.size, 1024)
        bundle.cipher.encryptBlockInPlace(c0, 0)
        bundle.cipher.encryptBlockInPlace(c1, 1)

        val biOut = ByteArrayOutputStream(128)
        biOut.write(bundle.dataHash)
        val be4 = ByteArray(4)
        val be8 = ByteArray(8)
        CryptoLz4.writeI32Be(be4, 0, 2); biOut.write(be4)
        for ((uSz, cSz, fl) in listOf(
            Triple(u0, c0.size, bundle.blocks[0].flags),
            Triple(u1, c1.size, bundle.blocks[1].flags),
        )) {
            CryptoLz4.writeI32Be(be4, 0, uSz); biOut.write(be4)
            CryptoLz4.writeI32Be(be4, 0, cSz); biOut.write(be4)
            biOut.write((fl ushr 8) and 0xFF)
            biOut.write(fl and 0xFF)
        }
        CryptoLz4.writeI32Be(be4, 0, bundle.dirs.size); biOut.write(be4)
        for (d in bundle.dirs) {
            CryptoLz4.writeI64Be(be8, 0, d.offset); biOut.write(be8)
            CryptoLz4.writeI64Be(be8, 0, d.size); biOut.write(be8)
            CryptoLz4.writeI32Be(be4, 0, d.flags); biOut.write(be4)
            biOut.write(d.name.toByteArray(Charsets.UTF_8))
            biOut.write(0)
        }
        val newBiPlain = biOut.toByteArray()
        val newBiComp = CryptoLz4.lz4Compress(newBiPlain, 256)
        if (bundle.blocksInfoOff + newBiComp.size > bundle.dataStart) {
            error("Compressed blocksInfo overflowed dataStart")
        }

        val newSub = ByteArray(subSize)
        System.arraycopy(bundle.headerPrefix, 0, newSub, 0, bundle.headerPrefix.size)
        CryptoLz4.writeI64Be(newSub, bundle.fsizeOff, subSize.toLong())
        CryptoLz4.writeI32Be(newSub, bundle.cinfoOff, newBiComp.size)
        CryptoLz4.writeI32Be(newSub, bundle.cinfoOff + 4, newBiPlain.size)
        System.arraycopy(newBiComp, 0, newSub, bundle.blocksInfoOff, newBiComp.size)
        System.arraycopy(c0, 0, newSub, bundle.dataStart, c0.size)
        System.arraycopy(c1, 0, newSub, bundle.dataStart + c0.size, c1.size)

        val outBuf = origBuf.copyOf()
        System.arraycopy(newSub, 0, outBuf, subOffset, subSize)

        // Round-trip verify decrypted SerializedFile
        val vBundle = UnityCnBundle.parse(outBuf, subOffset)
        val vSf = vBundle.readAll()
        if (!vSf.contentEquals(newSf)) error("Static UI round-trip verification failed")

        return Pair(outBuf, replaced)
    }

    // ---------- BestHTTP HTTPCache + Library v3 (Cached News) ----------

    private data class NewsLibEntry(
        val hash: String?,
        val hexName: String,
        val bodyLenOff: Int,
        val bodyLen: Int,
    )

    private fun read7BitStr(buf: ByteArray, pRef: IntArray): String {
        var len = 0
        var shift = 0
        while (true) {
            if (pRef[0] >= buf.size) error("Unexpected EOF reading 7-bit string len")
            val b = buf[pRef[0]++].toInt() and 0xFF
            len = len or ((b and 0x7F) shl shift)
            if ((b and 0x80) == 0) break
            shift += 7
            if (shift > 28) error("Corrupt 7-bit string len in Library")
        }
        if (pRef[0] + len > buf.size) error("Unexpected EOF reading 7-bit string payload")
        val s = String(buf, pRef[0], len, Charsets.UTF_8)
        pRef[0] += len
        return s
    }

    private fun extractAnnouncementHash(url: String): String? {
        val lower = url.lowercase()
        val marker = "announcement/prod/"
        val idx = lower.indexOf(marker)
        if (idx < 0) return null
        val rest = lower.substring(idx + marker.length)
        val end = rest.indexOf("/index.html")
        if (end != 32) return null
        val h = rest.substring(0, 32)
        return if (h.all { it in '0'..'9' || it in 'a'..'f' }) h else null
    }

    private fun parseHttpCacheLibrary(buf: ByteArray): List<NewsLibEntry> {
        if (buf.size < 16) error("Library file too small")
        val ver = CryptoLz4.readI32Le(buf, 0)
        if (ver != 3) error("Unsupported HTTP cache Library version: $ver")
        val cnt = CryptoLz4.readI32Le(buf, 12)
        val pRef = intArrayOf(16)
        val out = ArrayList<NewsLibEntry>(cnt)
        for (i in 0 until cnt) {
            val url = read7BitStr(buf, pRef)
            if (pRef[0] + 37 > buf.size) error("Library entry truncated")
            pRef[0] += 8 // LastAccess
            val bodyLenOff = pRef[0]
            val bodyLen = CryptoLz4.readI32Le(buf, pRef[0])
            pRef[0] += 4
            pRef[0] += 17 // ver3 reserved
            var mappedIdx = 0L
            for (b in 0 until 8) {
                mappedIdx = mappedIdx or ((buf[pRef[0] + b].toLong() and 0xFFL) shl (b * 8))
            }
            pRef[0] += 8
            read7BitStr(buf, pRef) // ETag
            read7BitStr(buf, pRef) // Last-Modified
            if (pRef[0] + 41 > buf.size) error("Library entry truncated in footer")
            pRef[0] += 41
            out.add(NewsLibEntry(extractAnnouncementHash(url), mappedIdx.toString(16).uppercase(), bodyLenOff, bodyLen))
        }
        if (pRef[0] != buf.size) error("Library parse length mismatch (${pRef[0]} != ${buf.size})")
        return out
    }

    private fun findHttpHeaderEnd(buf: ByteArray): Int {
        for (i in 0..buf.size - 4) {
            if (buf[i] == 13.toByte() && buf[i + 1] == 10.toByte() && buf[i + 2] == 13.toByte() && buf[i + 3] == 10.toByte()) {
                return i
            }
        }
        return -1
    }

    private fun updateHttpContentLength(hdrBytes: ByteArray, newLen: Int): ByteArray {
        val hdrStr = String(hdrBytes, Charsets.ISO_8859_1)
        val lines = hdrStr.split("\r\n").toMutableList()
        val isGzip = lines.any { it.lowercase().startsWith("content-encoding:") && it.lowercase().contains("gzip") }
        if (isGzip) return hdrBytes
        for (i in lines.indices) {
            if (lines[i].lowercase().startsWith("content-length:")) {
                lines[i] = "Content-Length: $newLen"
            }
        }
        return lines.joinToString("\r\n").toByteArray(Charsets.ISO_8859_1)
    }

    fun patchNewsCacheDir(
        cacheBase: File,
        loader: PayloadLoader,
        lang: PatchLang,
        log: (String, LogLevel) -> Unit,
        onProgress: (Int, Int, String) -> Unit,
    ): Pair<Int, Int> {
        val libFile = File(cacheBase, "Library")
        val hcDir = File(cacheBase, "HTTPCache")
        if (!libFile.isFile || !hcDir.isDirectory) {
            error("News cache not found. Open in-game News once, close the game, then try again.")
        }
        val dictName = if (lang == PatchLang.ID) "news_id.bin.gz" else "news_en.bin.gz"
        val dict = loadStringDict(loader.load(dictName))
        val bakDir = File(cacheBase, "_NewsPatchBackup")
        val bakHc = File(bakDir, "HTTPCache")
        val bakLib = File(bakDir, "Library")
        bakHc.mkdirs()

        if (!bakLib.isFile) {
            libFile.copyTo(bakLib, overwrite = true)
            log("  Backed up original News Library.", LogLevel.OK)
        }
        val libBytes = bakLib.readBytes()
        val entries = parseHttpCacheLibrary(libBytes)
        val newsEntries = entries.filter { it.hash != null && dict.containsKey(it.hash) }
        if (newsEntries.isEmpty()) {
            error("No matching cached announcement pages found in Library. Open the in-game News screen once first.")
        }

        val outLib = libBytes.copyOf()
        var patched = 0
        for ((idx, ent) in newsEntries.withIndex()) {
            val hash = ent.hash!!
            val origFile = File(hcDir, ent.hexName)
            val bakFile = File(bakHc, ent.hexName)
            if (!bakFile.isFile) {
                if (!origFile.isFile) continue
                origFile.copyTo(bakFile, overwrite = true)
            }
            val rawEntry = bakFile.readBytes()
            val hdrEnd = findHttpHeaderEnd(rawEntry)
            if (hdrEnd < 0) continue
            val hdr = rawEntry.copyOfRange(0, hdrEnd)
            val htmlBytes = dict[hash]!!.toByteArray(Charsets.UTF_8)
            val newHdr = updateHttpContentLength(hdr, htmlBytes.size)
            val out = ByteArrayOutputStream(newHdr.size + 4 + htmlBytes.size)
            out.write(newHdr)
            out.write("\r\n\r\n".toByteArray(Charsets.US_ASCII))
            out.write(htmlBytes)
            origFile.writeBytes(out.toByteArray())
            CryptoLz4.writeI32Le(outLib, ent.bodyLenOff, htmlBytes.size)
            patched++
            onProgress(patched, newsEntries.size, "Translating News [${lang.shortCode}]: $patched / ${newsEntries.size}")
            log("  [${lang.shortCode}] HC/${ent.hexName} (${hash.take(8)}): ${ent.bodyLen} B -> ${htmlBytes.size} B", LogLevel.DIFF)
        }
        libFile.writeBytes(outLib)
        return Pair(patched, newsEntries.size)
    }

    fun rollbackNewsCacheDir(cacheBase: File, log: (String, LogLevel) -> Unit): Int {
        val bakDir = File(cacheBase, "_NewsPatchBackup")
        val bakLib = File(bakDir, "Library")
        val bakHc = File(bakDir, "HTTPCache")
        if (!bakLib.isFile) return 0
        val hcDir = File(cacheBase, "HTTPCache")
        hcDir.mkdirs()
        var count = 0
        bakHc.listFiles()?.forEach { f ->
            if (f.isFile) {
                f.copyTo(File(hcDir, f.name), overwrite = true)
                count++
            }
        }
        bakLib.copyTo(File(cacheBase, "Library"), overwrite = true)
        log("  Restored News Library + $count original HTML announcements.", LogLevel.INFO)
        return count
    }

    fun parseBestManifestRev(listing: String): String {
        var bestNum = -1L
        var bestStr = ""
        for (tok in listing.split(Regex("\\s+"))) {
            if (!tok.endsWith(".bytes")) continue
            val stem = tok.removeSuffix(".bytes")
            if (!stem.contains("_package_")) continue
            val rev = stem.substringAfterLast('_', "")
            if (rev.isNotEmpty() && rev.all { it.isDigit() }) {
                val n = rev.toLongOrNull() ?: continue
                if (n > bestNum) {
                    bestNum = n
                    bestStr = rev
                }
            }
        }
        return bestStr
    }
}
