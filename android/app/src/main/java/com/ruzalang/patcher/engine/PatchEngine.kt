package com.ruzalang.patcher.engine

import java.io.ByteArrayOutputStream
import java.io.File
import java.io.RandomAccessFile

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

data class JpKey(val hash: Long, val len: Int)
data class DiscoveredVString(val fileName: String, val entryCount: Int)
data class DiscoveredStaticUi(val fileName: String, val subOffset: Int, val subSize: Int)

object PatchEngine {
    const val ANDROID_PACKAGE_ID = "com.Yostar.AzurpromiliaJP"
    const val ANDROID_EXPECTED_VERSION = "1.0.0"
    const val ANDROID_EXPECTED_RES_REV = "3273851"
    const val VSTRING_STRINGS = 76180
    const val STATIC_UI_STRINGS = 3135
    const val ANDROID_TOTAL_STRINGS = VSTRING_STRINGS + STATIC_UI_STRINGS // 79,315

    const val REMOTE_FILES_ROOT = "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files"
    const val REMOTE_RAW_BUNDLE_DIR =
        "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files/.res/raw_base_package/BundleFiles"
    const val REMOTE_DEFAULT_BUNDLE_DIR =
        "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files/.res/default_package/BundleFiles"
    const val REMOTE_RAW_MANIFEST_DIR =
        "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files/.res/raw_base_package/ManifestFiles"
    const val REMOTE_DEFAULT_MANIFEST_DIR =
        "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files/.res/default_package/ManifestFiles"

    val KNOWN_ANDROID_STATIC_UI_NAMES = listOf("nibdvaxcnfh3en5bnvhtw.r")

    // Known SHA-256 sets (supplementing dynamic _ruza_state.json hotfix tracking)
    val ANDROID_VSTRING_ORIG_SHAS = setOf(
        "C6BA69F2403102A9D4DBF2B3AB079ABBE7DE25896ABDB3969DD5B7988A2DB013", // r3219596
    )
    val ANDROID_STATIC_UI_ORIG_SHAS = setOf(
        "53A36C59B4ABABF9457ED37B195092DCAFD491C2D95D9BA75A61998C0138D22A", // r3219596
    )
    val ANDROID_VSTRING_KNOWN_EN_SHAS = setOf(
        "0739151C287863A4CD45150CC85DE3DDFDD3A7587FB159437F3F5FEED513A4E5", // v0.8.x r3219596
        "3216E4E2A56F2BC36EAA2FFF79B203C854FA834A93A97C2CA218B40E909FF5F1", // v0.9.0 r3219596
        "2124FBF99624837DD6C36F0712EDB573ACFDE62DAC304FEF87678E6B91EDE1FF", // v0.9.1 r3219596
    )
    val ANDROID_VSTRING_KNOWN_ID_SHAS = setOf(
        "BD9A20B6BE6F9228E12F435F72374386F8C15D270E30B442927401A2EB36A0AF", // v0.8.x r3219596
        "EBC5EE0155910A58D3B62AB978A886CCD1BA4FA68F4CE929B7763E32454531DE", // v0.9.0 r3219596
        "43DA80F93DACA269313BCBF0B6D14BC3685F25AE479806419DFF848E24AD7150", // v0.9.1 r3219596
    )
    val ANDROID_STATIC_UI_KNOWN_EN_SHAS = setOf(
        "2A4428AE21009DB19621A123B1A2561ED1F83A685B4C9160B94BA74CD50E8DAC", // v0.9.0 r3219596
        "EB18599A3513CDF558F4716E0B70DEA233CAC9076D50BA6EFEB0D7958C87F01B", // v0.9.1 r3219596
    )
    val ANDROID_STATIC_UI_KNOWN_ID_SHAS = setOf(
        "0017773EB20E5B56D3E02179A9F94A6FBC862B48CFD9FF7CF443CCAE9E6A64B2", // v0.8.x r3219596
        "60F0B86F98FCA84D46AE1AD7531510320812972D5371F80FC358860B660BC308", // v0.9.0 r3219596
        "10939AD972515B9A6DF118AC5D7ADA99BBA7F0C0FBF54FCAE79698987B7EE59B", // v0.9.1 r3219596
    )

    fun probeVStringFile(file: File, key4096: ByteArray): Int? {
        if (!file.isFile) return null
        val fsize = file.length()
        if (fsize < 50_000_000L || fsize > Int.MAX_VALUE.toLong()) return null
        return try {
            RandomAccessFile(file, "r").use { raf ->
                val h1 = ByteArray(24)
                raf.seek(4120L)
                raf.readFully(h1)
                CryptoLz4.xorMMapRangeInPlace(h1, 0, h1.size, 4120, key4096)
                val hashBytes = CryptoLz4.readI32Le(h1, 0)
                val hashStart = CryptoLz4.readI32Le(h1, 4)
                if (hashStart != 4120 || hashBytes <= 0) return null
                val strSecOff = 4120L + hashBytes.toLong()
                if (strSecOff + 20L > fsize) return null
                val sh = ByteArray(20)
                raf.seek(strSecOff)
                raf.readFully(sh)
                CryptoLz4.xorMMapRangeInPlace(sh, 0, sh.size, strSecOff.toInt(), key4096)
                val count = CryptoLz4.readI32Le(sh, 16)
                if (count < 500_000 || strSecOff + 20L + count.toLong() * 4L >= fsize) return null
                count
            }
        } catch (_: Throwable) {
            null
        }
    }

    fun discoverVStringInDir(dir: File, key4096: ByteArray): DiscoveredVString? {
        if (!dir.isDirectory) return null
        val files = dir.listFiles() ?: return null
        var bestName: String? = null
        var bestCount = -1
        var bestMtime = -1L
        for (f in files) {
            if (!f.isFile || !f.name.endsWith(".r") || f.length() < 50_000_000L) continue
            val cnt = probeVStringFile(f, key4096) ?: continue
            val mt = f.lastModified()
            if (cnt > bestCount || (cnt == bestCount && mt > bestMtime)) {
                bestCount = cnt
                bestMtime = mt
                bestName = f.name
            }
        }
        return bestName?.let { DiscoveredVString(it, bestCount) }
    }

    fun discoverStaticUiInDir(dir: File, preferredNames: List<String> = KNOWN_ANDROID_STATIC_UI_NAMES): DiscoveredStaticUi? {
        if (!dir.isDirectory) return null
        val checked = HashSet<String>()
        for (name in preferredNames) {
            if (name.isBlank() || !checked.add(name)) continue
            val f = File(dir, name)
            if (!f.isFile || f.length() !in 100_000L..30_000_000L) continue
            try {
                val found = CryptoLz4.findStaticUiSubBundle(f.readBytes())
                if (found != null) return DiscoveredStaticUi(name, found.first, found.second)
            } catch (_: Throwable) {
            }
        }
        val files = dir.listFiles()?.filter {
            it.isFile && it.name.endsWith(".r") && !checked.contains(it.name) && it.length() in 100_000L..30_000_000L
        } ?: return null
        val sorted = files.sortedWith(
            compareBy<File> { it.length() !in 2_500_000L..10_000_000L }
                .thenByDescending { it.lastModified() },
        )
        for (f in sorted) {
            try {
                val found = CryptoLz4.findStaticUiSubBundle(f.readBytes())
                if (found != null) return DiscoveredStaticUi(f.name, found.first, found.second)
            } catch (_: Throwable) {
            }
        }
        return null
    }

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

    fun loadRzv2Table(gz: ByteArray): Pair<HashMap<JpKey, ByteArray>, HashMap<Int, JpKey>> {
        val raw = CryptoLz4.gunzip(gz)
        if (raw.size < 8 ||
            raw[0] != 'R'.code.toByte() ||
            raw[1] != 'Z'.code.toByte() ||
            raw[2] != 'V'.code.toByte() ||
            raw[3] != '2'.code.toByte()
        ) {
            error("Corrupt RZV2 VStringPool payload header")
        }
        val count = CryptoLz4.readI32Le(raw, 4)
        val byHash = HashMap<JpKey, ByteArray>(count * 4 / 3 + 16)
        val pcIdToJp = HashMap<Int, JpKey>(count * 4 / 3 + 16)
        var p = 8
        for (i in 0 until count) {
            if (p + 20 > raw.size) error("Truncated RZV2 record header")
            val pcId = CryptoLz4.readI32Le(raw, p)
            val jpHash = CryptoLz4.readI64Le(raw, p + 4)
            val jpLen = CryptoLz4.readI32Le(raw, p + 12)
            val trLen = CryptoLz4.readI32Le(raw, p + 16)
            p += 20
            if (p + trLen > raw.size) error("Truncated RZV2 record payload")
            val key = JpKey(jpHash, jpLen)
            byHash[key] = raw.copyOfRange(p, p + trLen)
            pcIdToJp[pcId] = key
            p += trLen
        }
        return Pair(byHash, pcIdToJp)
    }

    fun buildVStringHashTable(
        loader: PayloadLoader,
        lang: PatchLang,
        custom: CustomPack? = null,
    ): HashMap<JpKey, ByteArray> {
        val (byHash, pcIdToJp) = when {
            custom != null -> when (custom.base) {
                CustomBase.EN -> loadRzv2Table(loader.load("vstring_en.bin.gz"))
                CustomBase.ID -> loadRzv2Table(loader.load("vstring_id.bin.gz"))
                CustomBase.JP -> {
                    val (_, idMap) = loadRzv2Table(loader.load("vstring_en.bin.gz"))
                    Pair(HashMap<JpKey, ByteArray>(custom.vstring.size * 4 / 3 + 16), idMap)
                }
            }
            lang == PatchLang.ID -> loadRzv2Table(loader.load("vstring_id.bin.gz"))
            else -> loadRzv2Table(loader.load("vstring_en.bin.gz"))
        }
        if (custom != null) {
            for ((pcId, bytes) in custom.vstring) {
                val fp = pcIdToJp[pcId]
                if (fp != null) byHash[fp] = bytes
            }
        }
        return byHash
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
     * Low-RAM in-place universal VStringPool exact-size repacker (~145 MB peak RAM for 131 MB bundle).
     * Matches strings by (FNV-1a64(orig_jp_bytes), orig_jp_len) + dynamic contiguous spans (MAX_GAP=4096),
     * producing 100% bit-for-bit identical output to the desktop engine across all hotfixes!
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
        onSub(0, "Loading universal $activeCode dictionary (RZV2)...")
        val key4096 = loader.load("mmap_key_4096.bin")
        val byHash = buildVStringHashTable(loader, lang, custom)
        log("  Loaded ${byHash.size} universal $activeCode VStringPool hash entries.", LogLevel.INFO)

        onSub(0, "Decrypting VStringPool (MMap)...")
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
        if (strSecOff + 20 + count.toLong() * 4L > buf.size.toLong()) {
            error("VStringPool offset table out of bounds (count=$count)")
        }
        log("  Decrypted VStringPool: $count total strings.", LogLevel.INFO)

        onSub(0, "Matching $activeCode strings by content hash...")
        val matchedIds = ArrayList<Int>(minOf(byHash.size, count))
        val matchedMap = HashMap<Int, ByteArray>(minOf(byHash.size, count) * 4 / 3 + 16)
        for (id in 0 until count) {
            val oAbs = CryptoLz4.readI32Le(buf, strSecOff + 20 + id * 4)
            if (oAbs < strSecOff || oAbs + 4 > buf.size) error("VStringPool offset OOB at #$id")
            val oLen = CryptoLz4.readI32Le(buf, oAbs)
            if (oLen < 0) continue
            if (oAbs + 4 + oLen > buf.size) error("VStringPool payload OOB at #$id")
            val h = CryptoLz4.fnv1a64(buf, oAbs + 4, oLen)
            val tr = byHash[JpKey(h, oLen)]
            if (tr != null) {
                matchedIds.add(id)
                matchedMap[id] = tr
            }
        }
        if (matchedIds.isEmpty()) error("No matching VStringPool entries found in this bundle.")
        log("  Matched ${matchedIds.size} strings dynamically across pool (count=$count).", LogLevel.INFO)

        // Group matched IDs into contiguous repackable spans (MAX_GAP=4096, MIN_SPAN_LEN=2)
        val maxGap = 4096
        val minSpanLen = 2
        val spans = ArrayList<Pair<Int, Int>>(8)
        val inplaceIds = ArrayList<Int>(8)
        var s = matchedIds[0]
        var e = matchedIds[0]
        for (i in 1 until matchedIds.size) {
            val id = matchedIds[i]
            if (id - e <= maxGap) {
                e = id
            } else {
                if (e - s + 1 >= minSpanLen) spans.add(Pair(s, e)) else inplaceIds.add(s)
                s = id
                e = id
            }
        }
        if (e - s + 1 >= minSpanLen) spans.add(Pair(s, e)) else inplaceIds.add(s)

        var rewritten = 0
        val totalTr = matchedIds.size
        val len4 = ByteArray(4)

        for ((spanIdx, span) in spans.withIndex()) {
            val (start, end) = span
            val spanStartAbs = CryptoLz4.readI32Le(buf, strSecOff + 20 + start * 4)
            val spanEndAbs = if (end + 1 < count) {
                CryptoLz4.readI32Le(buf, strSecOff + 20 + (end + 1) * 4)
            } else {
                buf.size
            }
            if (spanEndAbs < spanStartAbs) error("Non-monotonic span offsets [$start..$end]")
            val capacity = spanEndAbs - spanStartAbs
            val packed = ByteArrayOutputStream(capacity)
            val newOffsets = IntArray(end - start + 1)
            var cursor = spanStartAbs

            for (id in start..end) {
                newOffsets[id - start] = cursor
                val tr = matchedMap[id]
                if (tr != null) {
                    rewritten++
                    if (rewritten % 2500 == 0) {
                        onSub(rewritten, "Rewriting VStringPool [$activeCode / span ${spanIdx + 1}]: $rewritten / $totalTr")
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
            if (used > capacity) error("Span #${spanIdx + 1} [$start..$end] overflowed by ${used - capacity} bytes")

            for (i in newOffsets.indices) {
                CryptoLz4.writeI32Le(buf, strSecOff + 20 + (start + i) * 4, newOffsets[i])
            }
            val packedBytes = packed.toByteArray()
            System.arraycopy(packedBytes, 0, buf, spanStartAbs, used)
            if (used < capacity) {
                buf.fill(0, spanStartAbs + used, spanStartAbs + capacity)
            }
            log("  Repacked span #${spanIdx + 1} [#$start..#$end]: $used/$capacity B (free slack ${capacity - used} B)", LogLevel.DIFF)
        }

        var skippedInplace = 0
        for (id in inplaceIds) {
            val oAbs = CryptoLz4.readI32Le(buf, strSecOff + 20 + id * 4)
            val oLen = CryptoLz4.readI32Le(buf, oAbs)
            val payload = matchedMap[id]!!
            if (payload.size > oLen) {
                matchedMap.remove(id)
                skippedInplace++
                continue
            }
            CryptoLz4.writeI32Le(buf, oAbs, payload.size)
            System.arraycopy(payload, 0, buf, oAbs + 4, payload.size)
            if (payload.size < oLen) {
                buf.fill(0, oAbs + 4 + payload.size, oAbs + 4 + oLen)
            }
            rewritten++
        }
        if (skippedInplace > 0) {
            log("  Skipped $skippedInplace isolated singleton string(s) exceeding in-place byte length.", LogLevel.WARN)
        }

        // Verify all applied strings before re-encryption
        onSub(rewritten, "Re-encrypting & verifying $rewritten VStringPool strings...")
        for ((id, expected) in matchedMap) {
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

    private fun findObj0SizeOffset(sf: ByteArray, dataOff: Int, origMbSize: Int): Int {
        if (3220 + 4 <= dataOff && CryptoLz4.readI32Le(sf, 3220) == origMbSize) {
            return 3220
        }
        var o = 8
        while (o + 4 <= dataOff) {
            if (CryptoLz4.readI32Le(sf, o) == origMbSize) {
                var allZero = true
                for (z in o - 8 until o) {
                    if (sf[z] != 0.toByte()) {
                        allZero = false
                        break
                    }
                }
                if (allZero) return o
            }
            o += 4
        }
        var o2 = maxOf(0, dataOff - 512) and 3.inv()
        while (o2 + 4 <= dataOff) {
            if (CryptoLz4.readI32Le(sf, o2) == origMbSize) return o2
            o2 += 4
        }
        return -1
    }

    fun patchAndroidStaticUiBytes(
        origBuf: ByteArray,
        subOffset: Int,
        loader: PayloadLoader,
        lang: PatchLang,
        custom: CustomPack? = null,
        onSub: (Int, String) -> Unit,
    ): Pair<ByteArray, Int> {
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
        val subSize = bundle.fsize
        if (subOffset + subSize > origBuf.size) {
            error("Static UI sub-bundle offset out of range ($subOffset + $subSize > ${origBuf.size})")
        }
        if (bundle.blocks.isEmpty()) error("Static UI sub-bundle has 0 blocks")
        val origSf = bundle.readAll()

        val marker = "ast_staticTextKeyConfig".toByteArray(Charsets.US_ASCII)
        val markerPos = CryptoLz4.findBytes(origSf, marker)
        if (markerPos < 32 || CryptoLz4.readI32Le(origSf, markerPos - 4) != marker.size) {
            error("Unexpected ast_staticTextKeyConfig MonoBehaviour layout")
        }
        val mbNameOff = markerPos - 4
        val dataOff = mbNameOff - 28
        val nameEnd = mbNameOff + 4 + align4(marker.size)

        var cur = nameEnd
        val langLen = CryptoLz4.readI32Le(origSf, cur); cur += 4
        val langTag = String(origSf, cur, langLen, Charsets.UTF_8)
        cur = align4(cur + langLen)
        val pairCount = CryptoLz4.readI32Le(origSf, cur); cur += 4

        val newMb = ByteArrayOutputStream(maxOf(16384, origSf.size - dataOff))
        newMb.write(origSf, dataOff, nameEnd - dataOff)
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
                    onSub(replaced, "Rewriting Static UI KeyConfig [$activeCode]: $replaced / $STATIC_UI_STRINGS")
                }
                tr
            } else {
                origV
            }
            writeStr(k)
            writeStr(v)
        }

        val origMbSize = cur - dataOff
        val obj0SizeOff = findObj0SizeOffset(origSf, dataOff, origMbSize)
        if (obj0SizeOff < 0) error("Could not locate MonoBehaviour ObjectInfo size=$origMbSize in Static UI SF")

        val newMbBytes = newMb.toByteArray()
        if (newMbBytes.size > origMbSize) {
            error("Rebuilt Static UI exceeds slot (${newMbBytes.size} > $origMbSize)")
        }

        val newSf = origSf.copyOf()
        newSf.fill(0, dataOff, dataOff + origMbSize)
        System.arraycopy(newMbBytes, 0, newSf, dataOff, newMbBytes.size)
        CryptoLz4.writeI32Le(newSf, obj0SizeOff, newMbBytes.size)

        onSub(replaced, "Compressing (LZ4HC exact) & UnityCN encrypting Static UI...")
        val totalBudget = subSize - bundle.dataStart
        val compBlocks = ArrayList<ByteArray>(bundle.blocks.size)
        var sfPos = 0
        var usedComp = 0
        val lastIdx = bundle.blocks.size - 1
        for ((idx, blk) in bundle.blocks.withIndex()) {
            val slice = newSf.copyOfRange(sfPos, sfPos + blk.uSize)
            sfPos += blk.uSize
            val c = if (idx < lastIdx) {
                CryptoLz4.lz4Compress(slice, 1024)
            } else {
                if (usedComp >= totalBudget) {
                    error("Static UI compressed blocks exceeded budget ($usedComp >= $totalBudget)")
                }
                CryptoLz4.lz4CompressExact(slice, totalBudget - usedComp, 1024)
            }
            if ((blk.flags and 0x100) != 0) {
                bundle.cipher.encryptBlockInPlace(c, idx)
            }
            usedComp += c.size
            compBlocks.add(c)
        }

        val biOut = ByteArrayOutputStream(128)
        biOut.write(bundle.dataHash)
        val be4 = ByteArray(4)
        val be8 = ByteArray(8)
        CryptoLz4.writeI32Be(be4, 0, bundle.blocks.size); biOut.write(be4)
        for (i in bundle.blocks.indices) {
            val blk = bundle.blocks[i]
            val c = compBlocks[i]
            CryptoLz4.writeI32Be(be4, 0, blk.uSize); biOut.write(be4)
            CryptoLz4.writeI32Be(be4, 0, c.size); biOut.write(be4)
            biOut.write((blk.flags ushr 8) and 0xFF)
            biOut.write(blk.flags and 0xFF)
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
        var wp = bundle.dataStart
        for (c in compBlocks) {
            System.arraycopy(c, 0, newSub, wp, c.size)
            wp += c.size
        }

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
