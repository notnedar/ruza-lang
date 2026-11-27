package com.ruzalang.patcher.storage

import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import androidx.documentfile.provider.DocumentFile
import com.ruzalang.patcher.IPatcherFileService
import com.ruzalang.patcher.engine.CryptoLz4
import com.ruzalang.patcher.engine.CustomPack
import com.ruzalang.patcher.engine.DiscoveredStaticUi
import com.ruzalang.patcher.engine.DiscoveredVString
import com.ruzalang.patcher.engine.LogLevel
import com.ruzalang.patcher.engine.PatchEngine
import com.ruzalang.patcher.engine.PatchLang
import com.ruzalang.patcher.engine.PayloadLoader
import org.json.JSONObject
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.nio.ByteBuffer
import java.security.MessageDigest

enum class AccessMode(val id: String, val label: String) {
    SHIZUKU("shizuku", "Shizuku (No Root)"),
    DIRECT("direct", "Direct Storage"),
    SAF("saf", "SAF Folder"),
    NONE("none", "Setup Required"),
}

data class AndroidTargetStatus(
    val id: String,
    val name: String,
    val subtitle: String,
    val stringCount: Int,
    val state: String, // "patched_en" | "patched_id" | "original" | "modified" | "missing"
    val hasBackup: Boolean,
    val sha256: String,
)

data class ScanSnapshot(
    val accessMode: AccessMode,
    val shizukuInstalled: Boolean,
    val shizukuRunning: Boolean,
    val shizukuGranted: Boolean,
    val gameInstalled: Boolean,
    val runningPids: List<Int>,
    val detectedVersion: String,
    val expectedVersion: String,
    val detectedResRev: String,
    val expectedResRev: String,
    val versionStatus: String, // "synced" | "mismatch"
    val versionReasons: List<String>,
    val targets: List<AndroidTargetStatus>,
    val newsCacheBytes: Long,
    val hasNewsBackup: Boolean,
    val backupDirPath: String,
)

class AssetPayloadLoader(private val context: Context) : PayloadLoader {
    override fun load(name: String): ByteArray {
        val assetName = if (name.endsWith(".gz")) name.removeSuffix(".gz") + ".gzp" else name
        context.assets.open("payloads/$assetName").use { return it.readBytes() }
    }
}

class StorageBridge(private val context: Context) {

    @Volatile
    var shizukuService: IPatcherFileService? = null

    @Volatile
    var safTreeUri: Uri? = null

    private val loader = AssetPayloadLoader(context)

    val appFilesRoot: File
        get() = (context.getExternalFilesDir(null) ?: context.filesDir).apply { mkdirs() }

    val backupDir: File
        get() = File(appFilesRoot, "backups").apply { mkdirs() }

    val stageDir: File
        get() = File(appFilesRoot, "stage").apply { mkdirs() }

    private val customPackFile: File
        get() = File(appFilesRoot, "custom_pack.json")

    private val customPackNameFile: File
        get() = File(appFilesRoot, "custom_pack.name")

    fun saveCustomPack(rawJson: String, fileName: String): CustomPack {
        val parsed = PatchEngine.parseCustomPackJson(rawJson, fileName)
        customPackFile.writeText(rawJson, Charsets.UTF_8)
        customPackNameFile.writeText(fileName, Charsets.UTF_8)
        return parsed
    }

    fun loadPersistedCustomPack(): CustomPack? {
        if (!customPackFile.isFile) return null
        return try {
            val fname = if (customPackNameFile.isFile) {
                customPackNameFile.readText(Charsets.UTF_8).trim().ifEmpty { "custom-pack.json" }
            } else {
                "custom-pack.json"
            }
            PatchEngine.parseCustomPackJson(customPackFile.readText(Charsets.UTF_8), fname)
        } catch (_: Throwable) {
            null
        }
    }

    fun clearCustomPack() {
        customPackFile.delete()
        customPackNameFile.delete()
    }

    private data class BundleState(
        var fileName: String = "",
        var subOffset: Int = 0,
        var lang: String = "",
        var customCode: String = "",
        var origSha: String = "",
        var patchedSha: String = "",
        var matchedCount: Int = 0,
    )

    private data class PatchState(
        var vstring: BundleState = BundleState(),
        var staticUi: BundleState = BundleState(),
    )

    private fun loadPatchState(): PatchState {
        val f = File(backupDir, "_ruza_state.json")
        if (!f.isFile) return PatchState()
        return try {
            val root = JSONObject(f.readText(Charsets.UTF_8))
            fun parseEntry(key: String): BundleState {
                val o = root.optJSONObject(key) ?: return BundleState()
                return BundleState(
                    fileName = o.optString("file_name", ""),
                    subOffset = o.optInt("sub_offset", 0),
                    lang = o.optString("lang", ""),
                    customCode = o.optString("custom_code", ""),
                    origSha = o.optString("orig_sha", ""),
                    patchedSha = o.optString("patched_sha", ""),
                    matchedCount = o.optInt("matched_count", 0),
                )
            }
            PatchState(parseEntry("vstring"), parseEntry("static_ui"))
        } catch (_: Throwable) {
            PatchState()
        }
    }

    private fun savePatchState(st: PatchState) {
        try {
            fun toJson(e: BundleState): JSONObject = JSONObject().apply {
                put("file_name", e.fileName)
                put("sub_offset", e.subOffset)
                put("lang", e.lang)
                put("custom_code", e.customCode)
                put("orig_sha", e.origSha)
                put("patched_sha", e.patchedSha)
                put("matched_count", e.matchedCount)
            }
            val root = JSONObject().apply {
                put("schema", 1)
                put("vstring", toJson(st.vstring))
                put("static_ui", toJson(st.staticUi))
            }
            File(backupDir, "_ruza_state.json").writeText(root.toString(2), Charsets.UTF_8)
        } catch (_: Throwable) {
        }
    }

    private fun stateTagFor(lang: PatchLang, custom: CustomPack?): String = when {
        custom != null -> "patched_custom"
        lang == PatchLang.ID -> "patched_id"
        else -> "patched_en"
    }

    private fun isDirectFsWritable(): Boolean {
        return try {
            val rawDir = File(PatchEngine.REMOTE_RAW_BUNDLE_DIR)
            val root = File(PatchEngine.REMOTE_FILES_ROOT)
            (rawDir.isDirectory && rawDir.canRead() && rawDir.canWrite()) ||
                (root.isDirectory && root.canRead() && root.canWrite())
        } catch (_: Throwable) {
            false
        }
    }

    private fun probeSafVString(doc: DocumentFile, key4096: ByteArray): Int? {
        val fsize = doc.length()
        if (fsize < 50_000_000L || fsize > Int.MAX_VALUE.toLong()) return null
        return try {
            context.contentResolver.openFileDescriptor(doc.uri, "r")?.use { pfd ->
                FileInputStream(pfd.fileDescriptor).channel.use { ch ->
                    fun readAt(off: Long, len: Int): ByteArray? {
                        val bb = ByteBuffer.allocate(len)
                        ch.position(off)
                        while (bb.hasRemaining()) {
                            val r = ch.read(bb)
                            if (r <= 0) return null
                        }
                        return bb.array()
                    }
                    val h1 = readAt(4120L, 24) ?: return null
                    CryptoLz4.xorMMapRangeInPlace(h1, 0, h1.size, 4120, key4096)
                    val hashBytes = CryptoLz4.readI32Le(h1, 0)
                    val hashStart = CryptoLz4.readI32Le(h1, 4)
                    if (hashStart != 4120 || hashBytes <= 0) return null
                    val strSecOff = 4120L + hashBytes.toLong()
                    if (strSecOff + 20L > fsize) return null
                    val sh = readAt(strSecOff, 20) ?: return null
                    CryptoLz4.xorMMapRangeInPlace(sh, 0, sh.size, strSecOff.toInt(), key4096)
                    val count = CryptoLz4.readI32Le(sh, 16)
                    if (count < 500_000 || strSecOff + 20L + count.toLong() * 4L >= fsize) return null
                    count
                }
            }
        } catch (_: Throwable) {
            null
        }
    }

    private fun discoverActiveVString(mode: AccessMode): DiscoveredVString? {
        val key4096 = loader.load("mmap_key_4096.bin")
        return when (mode) {
            AccessMode.SHIZUKU -> {
                val res = shizukuService?.findActiveVStringBundle(PatchEngine.REMOTE_RAW_BUNDLE_DIR, key4096).orEmpty()
                val parts = res.split('|')
                if (parts.size == 2 && parts[0].isNotBlank()) {
                    DiscoveredVString(parts[0], parts[1].toIntOrNull() ?: 0)
                } else {
                    null
                }
            }
            AccessMode.DIRECT -> PatchEngine.discoverVStringInDir(File(PatchEngine.REMOTE_RAW_BUNDLE_DIR), key4096)
            AccessMode.SAF -> {
                val dir = resolveSafPath(relFromRemote(PatchEngine.REMOTE_RAW_BUNDLE_DIR)) ?: return null
                var bestName: String? = null
                var bestCount = -1
                var bestMtime = -1L
                for (doc in dir.listFiles()) {
                    val name = doc.name ?: continue
                    if (!doc.isFile || !name.endsWith(".r") || doc.length() < 50_000_000L) continue
                    val cnt = probeSafVString(doc, key4096) ?: continue
                    val mt = doc.lastModified()
                    if (cnt > bestCount || (cnt == bestCount && mt > bestMtime)) {
                        bestCount = cnt
                        bestMtime = mt
                        bestName = name
                    }
                }
                bestName?.let { DiscoveredVString(it, bestCount) }
            }
            AccessMode.NONE -> null
        }
    }

    private fun discoverActiveStaticUi(mode: AccessMode, state: PatchState): DiscoveredStaticUi? {
        val preferred = state.staticUi.fileName
        return when (mode) {
            AccessMode.SHIZUKU -> {
                // Fast-path: if cached filename + subOffset exist and file is still present, zero scan!
                if (preferred.isNotBlank() && state.staticUi.subOffset > 0 &&
                    remoteExists(mode, "${PatchEngine.REMOTE_DEFAULT_BUNDLE_DIR}/$preferred")
                ) {
                    return DiscoveredStaticUi(preferred, state.staticUi.subOffset, 0)
                }
                val res = shizukuService?.findActiveStaticUiBundle(
                    PatchEngine.REMOTE_DEFAULT_BUNDLE_DIR,
                    preferred.ifBlank { null },
                ).orEmpty()
                val parts = res.split('|')
                if (parts.size >= 3 && parts[0].isNotBlank()) {
                    val found = DiscoveredStaticUi(
                        parts[0],
                        parts[1].toIntOrNull() ?: 0,
                        parts[2].toIntOrNull() ?: 0,
                    )
                    if (state.staticUi.fileName != found.fileName || state.staticUi.subOffset != found.subOffset) {
                        state.staticUi.fileName = found.fileName
                        state.staticUi.subOffset = found.subOffset
                        savePatchState(state)
                    }
                    found
                } else {
                    null
                }
            }
            AccessMode.DIRECT -> {
                val prefs = ArrayList<String>()
                if (preferred.isNotBlank()) prefs.add(preferred)
                prefs.addAll(PatchEngine.KNOWN_ANDROID_STATIC_UI_NAMES)
                val found = PatchEngine.discoverStaticUiInDir(File(PatchEngine.REMOTE_DEFAULT_BUNDLE_DIR), prefs)
                if (found != null && (state.staticUi.fileName != found.fileName || state.staticUi.subOffset != found.subOffset)) {
                    state.staticUi.fileName = found.fileName
                    state.staticUi.subOffset = found.subOffset
                    savePatchState(state)
                }
                found
            }
            AccessMode.SAF -> {
                val dir = resolveSafPath(relFromRemote(PatchEngine.REMOTE_DEFAULT_BUNDLE_DIR)) ?: return null
                if (preferred.isNotBlank() && state.staticUi.subOffset > 0 && dir.findFile(preferred)?.isFile == true) {
                    return DiscoveredStaticUi(preferred, state.staticUi.subOffset, 0)
                }
                val checked = HashSet<String>()
                fun probeDoc(doc: DocumentFile): DiscoveredStaticUi? {
                    val name = doc.name ?: return null
                    if (!doc.isFile || !name.endsWith(".r") || doc.length() !in 100_000L..30_000_000L) return null
                    val bytes = context.contentResolver.openInputStream(doc.uri)?.use { it.readBytes() } ?: return null
                    val found = CryptoLz4.findStaticUiSubBundle(bytes) ?: return null
                    return DiscoveredStaticUi(name, found.first, found.second)
                }
                val fastNames = ArrayList<String>()
                if (preferred.isNotBlank()) fastNames.add(preferred)
                fastNames.addAll(PatchEngine.KNOWN_ANDROID_STATIC_UI_NAMES)
                for (name in fastNames) {
                    if (!checked.add(name)) continue
                    val doc = dir.findFile(name) ?: continue
                    val found = probeDoc(doc)
                    if (found != null) {
                        state.staticUi.fileName = found.fileName
                        state.staticUi.subOffset = found.subOffset
                        savePatchState(state)
                        return found
                    }
                }
                val sorted = dir.listFiles()
                    .filter { it.isFile && (it.name?.endsWith(".r") == true) && !checked.contains(it.name) && it.length() in 100_000L..30_000_000L }
                    .sortedWith(
                        compareBy<DocumentFile> { it.length() !in 2_500_000L..10_000_000L }
                            .thenByDescending { it.lastModified() },
                    )
                for (doc in sorted) {
                    val found = probeDoc(doc)
                    if (found != null) {
                        state.staticUi.fileName = found.fileName
                        state.staticUi.subOffset = found.subOffset
                        savePatchState(state)
                        return found
                    }
                }
                null
            }
            AccessMode.NONE -> null
        }
    }

    private fun classifyBundleState(
        fileName: String,
        curSha: String,
        bakFile: File,
        entry: BundleState,
        knownOrig: Set<String>,
        knownEn: Set<String>,
        knownId: Set<String>,
    ): String {
        val up = curSha.uppercase()
        if (entry.fileName == fileName && entry.patchedSha.isNotEmpty() &&
            up.equals(entry.patchedSha, ignoreCase = true) && entry.lang.isNotEmpty()
        ) {
            return entry.lang
        }
        if (knownId.contains(up)) return "patched_id"
        if (knownEn.contains(up)) return "patched_en"
        if (knownOrig.contains(up)) return "original"
        if (entry.fileName == fileName && entry.origSha.isNotEmpty() && up.equals(entry.origSha, ignoreCase = true)) {
            return "original"
        }
        if (bakFile.isFile) {
            val bakSha = try { CryptoLz4.sha256File(bakFile) } catch (_: Throwable) { "" }
            if (bakSha.isNotEmpty() && up.equals(bakSha, ignoreCase = true)) return "original"
            return "modified"
        }
        // Newly discovered hotfix bundle (not yet backed up, not matching any known patched SHA) => Original JP!
        return "original"
    }

    fun resolveAccessMode(): AccessMode {
        val svc = shizukuService
        if (svc != null) {
            try {
                // Quick ping to confirm binder is alive
                svc.fileExists(PatchEngine.REMOTE_FILES_ROOT)
                return AccessMode.SHIZUKU
            } catch (_: Throwable) {
                shizukuService = null
            }
        }
        if (isDirectFsWritable()) return AccessMode.DIRECT
        if (safTreeUri != null && resolveSafRoot() != null) return AccessMode.SAF
        return AccessMode.NONE
    }

    private fun resolveSafRoot(): DocumentFile? {
        val uri = safTreeUri ?: return null
        val tree = DocumentFile.fromTreeUri(context, uri) ?: return null
        if (!tree.exists() || !tree.canRead()) return null
        // User may have picked either `com.Yostar.AzurpromiliaJP` or `com.Yostar.AzurpromiliaJP/files`
        val filesSub = tree.findFile("files")
        return if (filesSub != null && filesSub.isDirectory) filesSub else tree
    }

    private fun resolveSafPath(relSegments: List<String>): DocumentFile? {
        var cur = resolveSafRoot() ?: return null
        for (seg in relSegments) {
            cur = cur.findFile(seg) ?: return null
        }
        return cur
    }

    private fun relFromRemote(remotePath: String): List<String> {
        val trimmed = remotePath.removePrefix(PatchEngine.REMOTE_FILES_ROOT).trim('/')
        return if (trimmed.isEmpty()) emptyList() else trimmed.split('/')
    }

    private fun isPackageInstalledLocal(): Boolean {
        return try {
            if (Build.VERSION.SDK_INT >= 33) {
                context.packageManager.getPackageInfo(PatchEngine.ANDROID_PACKAGE_ID, PackageManager.PackageInfoFlags.of(0))
            } else {
                @Suppress("DEPRECATION")
                context.packageManager.getPackageInfo(PatchEngine.ANDROID_PACKAGE_ID, 0)
            }
            true
        } catch (_: Throwable) {
            false
        }
    }

    private fun getPackageVersionLocal(): String {
        return try {
            val pi = if (Build.VERSION.SDK_INT >= 33) {
                context.packageManager.getPackageInfo(PatchEngine.ANDROID_PACKAGE_ID, PackageManager.PackageInfoFlags.of(0))
            } else {
                @Suppress("DEPRECATION")
                context.packageManager.getPackageInfo(PatchEngine.ANDROID_PACKAGE_ID, 0)
            }
            pi.versionName ?: ""
        } catch (_: Throwable) {
            ""
        }
    }

    private fun remoteExists(mode: AccessMode, remotePath: String): Boolean = when (mode) {
        AccessMode.SHIZUKU -> shizukuService?.fileExists(remotePath) == true
        AccessMode.DIRECT -> File(remotePath).isFile
        AccessMode.SAF -> resolveSafPath(relFromRemote(remotePath))?.isFile == true
        AccessMode.NONE -> false
    }

    private fun remoteSize(mode: AccessMode, remotePath: String): Long = when (mode) {
        AccessMode.SHIZUKU -> shizukuService?.fileSize(remotePath) ?: -1L
        AccessMode.DIRECT -> File(remotePath).let { if (it.isFile) it.length() else -1L }
        AccessMode.SAF -> resolveSafPath(relFromRemote(remotePath))?.length() ?: -1L
        AccessMode.NONE -> -1L
    }

    private fun remoteSha256(mode: AccessMode, remotePath: String): String? {
        return when (mode) {
            AccessMode.SHIZUKU -> {
                val r = shizukuService?.sha256File(remotePath).orEmpty()
                if (r.length == 64 && !r.startsWith("ERR:")) r.uppercase() else null
            }
            AccessMode.DIRECT -> {
                val f = File(remotePath)
                if (f.isFile) CryptoLz4.sha256File(f) else null
            }
            AccessMode.SAF -> {
                val doc = resolveSafPath(relFromRemote(remotePath)) ?: return null
                val md = MessageDigest.getInstance("SHA-256")
                val buf = ByteArray(1024 * 1024)
                context.contentResolver.openInputStream(doc.uri)?.use { ins ->
                    while (true) {
                        val n = ins.read(buf)
                        if (n <= 0) break
                        md.update(buf, 0, n)
                    }
                } ?: return null
                CryptoLz4.hexUpper(md.digest())
            }
            AccessMode.NONE -> null
        }
    }

    private fun remoteListDir(mode: AccessMode, remoteDir: String): List<String> = when (mode) {
        AccessMode.SHIZUKU -> shizukuService?.listDir(remoteDir).orEmpty().lines().filter { it.isNotBlank() }
        AccessMode.DIRECT -> File(remoteDir).list()?.sorted().orEmpty()
        AccessMode.SAF -> resolveSafPath(relFromRemote(remoteDir))?.listFiles()?.mapNotNull { it.name }.orEmpty()
        AccessMode.NONE -> emptyList()
    }

    fun findRunningGamePids(mode: AccessMode): List<Int> {
        val raw = when (mode) {
            AccessMode.SHIZUKU -> shizukuService?.findRunningPids(PatchEngine.ANDROID_PACKAGE_ID).orEmpty()
            else -> ""
        }
        return raw.trim().split(Regex("\\s+")).mapNotNull { it.toIntOrNull() }
    }

    private fun copyFromRemote(mode: AccessMode, remotePath: String, localDst: File) {
        localDst.parentFile?.mkdirs()
        when (mode) {
            AccessMode.SHIZUKU -> {
                val res = shizukuService?.copyFileAtomic(remotePath, localDst.absolutePath)
                    ?: error("Shizuku service disconnected")
                if (!res.startsWith("OK:")) error("Pull failed: ${res.removePrefix("ERR:")}")
            }
            AccessMode.DIRECT -> {
                File(remotePath).copyTo(localDst, overwrite = true)
            }
            AccessMode.SAF -> {
                val doc = resolveSafPath(relFromRemote(remotePath))
                    ?: error("SAF file not found: $remotePath")
                val buf = ByteArray(1024 * 1024)
                context.contentResolver.openInputStream(doc.uri)?.use { ins ->
                    FileOutputStream(localDst).use { out ->
                        while (true) {
                            val n = ins.read(buf)
                            if (n <= 0) break
                            out.write(buf, 0, n)
                        }
                        out.fd.sync()
                    }
                } ?: error("Failed opening SAF input stream")
            }
            AccessMode.NONE -> error("No storage access method authorized")
        }
    }

    private fun pushToRemoteVerified(mode: AccessMode, localSrc: File, remotePath: String, expectedSha: String): String {
        when (mode) {
            AccessMode.SHIZUKU -> {
                val res = shizukuService?.copyFileAtomic(localSrc.absolutePath, remotePath)
                    ?: error("Shizuku service disconnected")
                if (!res.startsWith("OK:")) error("Push failed: ${res.removePrefix("ERR:")}")
                val sha = res.removePrefix("OK:")
                if (!sha.equals(expectedSha, ignoreCase = true)) {
                    error("SHA-256 verification mismatch ($sha != $expectedSha)")
                }
                return sha
            }
            AccessMode.DIRECT -> {
                val dst = File(remotePath)
                val tmp = File("${dst.absolutePath}.ruza_tmp")
                localSrc.copyTo(tmp, overwrite = true)
                val tmpSha = CryptoLz4.sha256File(tmp)
                if (!tmpSha.equals(expectedSha, ignoreCase = true)) {
                    tmp.delete()
                    error("Temp SHA-256 mismatch ($tmpSha != $expectedSha)")
                }
                if (!tmp.renameTo(dst)) {
                    tmp.copyTo(dst, overwrite = true)
                    tmp.delete()
                }
                val finalSha = CryptoLz4.sha256File(dst)
                if (!finalSha.equals(expectedSha, ignoreCase = true)) {
                    error("Final SHA-256 mismatch ($finalSha != $expectedSha)")
                }
                return finalSha
            }
            AccessMode.SAF -> {
                val rel = relFromRemote(remotePath)
                var doc = resolveSafPath(rel)
                if (doc == null && rel.size >= 2) {
                    val parent = resolveSafPath(rel.dropLast(1))
                        ?: error("SAF parent dir not found for $remotePath")
                    doc = parent.createFile("application/octet-stream", rel.last())
                }
                if (doc == null) error("SAF target not found: $remotePath")
                val buf = ByteArray(1024 * 1024)
                context.contentResolver.openOutputStream(doc.uri, "wt")?.use { out ->
                    FileInputStream(localSrc).use { ins ->
                        while (true) {
                            val n = ins.read(buf)
                            if (n <= 0) break
                            out.write(buf, 0, n)
                        }
                    }
                    out.flush()
                } ?: error("Failed opening SAF output stream")
                val finalSha = remoteSha256(mode, remotePath) ?: error("Post-write SAF hash failed")
                if (!finalSha.equals(expectedSha, ignoreCase = true)) {
                    error("Final SAF SHA-256 mismatch ($finalSha != $expectedSha)")
                }
                return finalSha
            }
            AccessMode.NONE -> error("No storage access method authorized")
        }
    }

    fun scan(
        shizukuInstalled: Boolean,
        shizukuRunning: Boolean,
        shizukuGranted: Boolean,
        log: (String, LogLevel) -> Unit,
    ): ScanSnapshot {
        val mode = resolveAccessMode()
        val gameInstalled = isPackageInstalledLocal() ||
            (mode == AccessMode.SHIZUKU && shizukuService?.isPackageInstalled(PatchEngine.ANDROID_PACKAGE_ID) == true)
        val detectedVersion = getPackageVersionLocal().ifEmpty {
            if (mode == AccessMode.SHIZUKU) shizukuService?.getPackageVersion(PatchEngine.ANDROID_PACKAGE_ID).orEmpty() else ""
        }
        val runningPids = if (gameInstalled) findRunningGamePids(mode) else emptyList()

        if (!gameInstalled || mode == AccessMode.NONE) {
            val reasons = mutableListOf<String>()
            if (!gameInstalled) {
                reasons.add("Target Android client is not installed on this device")
                log("Target Android client not detected on this device.", LogLevel.WARN)
            }
            if (mode == AccessMode.NONE) {
                reasons.add("Storage access to Android/data not authorized yet (authorize Shizuku or pick SAF folder)")
                log("Storage access needed: tap Setup Access (Shizuku or SAF).", LogLevel.WARN)
            }
            return ScanSnapshot(
                accessMode = mode,
                shizukuInstalled = shizukuInstalled,
                shizukuRunning = shizukuRunning,
                shizukuGranted = shizukuGranted,
                gameInstalled = gameInstalled,
                runningPids = runningPids,
                detectedVersion = detectedVersion,
                expectedVersion = PatchEngine.ANDROID_EXPECTED_VERSION,
                detectedResRev = "",
                expectedResRev = PatchEngine.ANDROID_EXPECTED_RES_REV,
                versionStatus = "mismatch",
                versionReasons = reasons,
                targets = emptyList(),
                newsCacheBytes = 0L,
                hasNewsBackup = File(backupDir, "NewsWork/_NewsPatchBackup/Library").isFile,
                backupDirPath = backupDir.absolutePath,
            )
        }

        val manifestListing = (remoteListDir(mode, PatchEngine.REMOTE_RAW_MANIFEST_DIR) +
            remoteListDir(mode, PatchEngine.REMOTE_DEFAULT_MANIFEST_DIR)).joinToString(" ")
        val detectedResRev = PatchEngine.parseBestManifestRev(manifestListing)
        val newsCacheBytes = remoteSize(mode, "${PatchEngine.REMOTE_FILES_ROOT}/Library").coerceAtLeast(0L)
        val hasNewsBackup = File(backupDir, "NewsWork/_NewsPatchBackup/Library").isFile

        val stateFile = loadPatchState()
        val discVs = discoverActiveVString(mode)
        val discUi = discoverActiveStaticUi(mode, stateFile)

        val targets = ArrayList<AndroidTargetStatus>(2)

        // [1/2] VStringPool (dynamically discovered!)
        if (discVs == null) {
            log("  [MISSING] VStringPool", LogLevel.WARN)
            targets.add(
                AndroidTargetStatus(
                    "vstring", "VStringPool", "Story, Quests, Skills, Kibo, Settings",
                    PatchEngine.VSTRING_STRINGS, "missing", false, "",
                ),
            )
        } else {
            val remote = "${PatchEngine.REMOTE_RAW_BUNDLE_DIR}/${discVs.fileName}"
            val bak = File(backupDir, "${discVs.fileName}.original")
            val hasBak = bak.isFile
            val sha = remoteSha256(mode, remote)
            if (sha == null) {
                log("  [MISSING] VStringPool [${discVs.fileName}]", LogLevel.WARN)
                targets.add(
                    AndroidTargetStatus(
                        "vstring", "VStringPool", "Story, Quests, Skills, Kibo, Settings",
                        PatchEngine.VSTRING_STRINGS, "missing", hasBak, "",
                    ),
                )
            } else {
                val st = classifyBundleState(
                    discVs.fileName, sha, bak, stateFile.vstring,
                    PatchEngine.ANDROID_VSTRING_ORIG_SHAS,
                    PatchEngine.ANDROID_VSTRING_KNOWN_EN_SHAS,
                    PatchEngine.ANDROID_VSTRING_KNOWN_ID_SHAS,
                )
                val dispCnt = if (stateFile.vstring.fileName == discVs.fileName && stateFile.vstring.matchedCount > 0) {
                    stateFile.vstring.matchedCount
                } else {
                    PatchEngine.VSTRING_STRINGS
                }
                val lvl = if (st.startsWith("patched_")) LogLevel.OK else LogLevel.INFO
                val lbl = when (st) {
                    "patched_en" -> "ENGLISH"
                    "patched_id" -> "INDONESIAN"
                    "patched_custom" -> "CUSTOM"
                    "original" -> "ORIGINAL JP"
                    else -> "MODIFIED"
                }
                log("  [$lbl] VStringPool [${discVs.fileName}] ($dispCnt strings, pool=${discVs.entryCount}, backup=${if (hasBak) "YES" else "NO"})", lvl)
                targets.add(
                    AndroidTargetStatus(
                        "vstring", "VStringPool", "Story, Quests, Skills, Kibo, Settings",
                        dispCnt, st, hasBak, sha,
                    ),
                )
            }
        }

        // [2/2] Static UI KeyConfig (dynamically discovered!)
        if (discUi == null) {
            log("  [MISSING] Static UI KeyConfig", LogLevel.WARN)
            targets.add(
                AndroidTargetStatus(
                    "static_ui", "Static UI KeyConfig", "Prefab Buttons, Headers, Notices",
                    PatchEngine.STATIC_UI_STRINGS, "missing", false, "",
                ),
            )
        } else {
            val remote = "${PatchEngine.REMOTE_DEFAULT_BUNDLE_DIR}/${discUi.fileName}"
            val bak = File(backupDir, "${discUi.fileName}.original")
            val hasBak = bak.isFile
            val sha = remoteSha256(mode, remote)
            if (sha == null) {
                log("  [MISSING] Static UI KeyConfig [${discUi.fileName}]", LogLevel.WARN)
                targets.add(
                    AndroidTargetStatus(
                        "static_ui", "Static UI KeyConfig", "Prefab Buttons, Headers, Notices",
                        PatchEngine.STATIC_UI_STRINGS, "missing", hasBak, "",
                    ),
                )
            } else {
                val st = classifyBundleState(
                    discUi.fileName, sha, bak, stateFile.staticUi,
                    PatchEngine.ANDROID_STATIC_UI_ORIG_SHAS,
                    PatchEngine.ANDROID_STATIC_UI_KNOWN_EN_SHAS,
                    PatchEngine.ANDROID_STATIC_UI_KNOWN_ID_SHAS,
                )
                val lvl = if (st.startsWith("patched_")) LogLevel.OK else LogLevel.INFO
                val lbl = when (st) {
                    "patched_en" -> "ENGLISH"
                    "patched_id" -> "INDONESIAN"
                    "patched_custom" -> "CUSTOM"
                    "original" -> "ORIGINAL JP"
                    else -> "MODIFIED"
                }
                log("  [$lbl] Static UI KeyConfig [${discUi.fileName}] (${PatchEngine.STATIC_UI_STRINGS} strings, off=${discUi.subOffset}, backup=${if (hasBak) "YES" else "NO"})", lvl)
                targets.add(
                    AndroidTargetStatus(
                        "static_ui", "Static UI KeyConfig", "Prefab Buttons, Headers, Notices",
                        PatchEngine.STATIC_UI_STRINGS, st, hasBak, sha,
                    ),
                )
            }
        }

        val reasons = mutableListOf<String>()
        if (detectedVersion.isNotEmpty() && detectedVersion != PatchEngine.ANDROID_EXPECTED_VERSION) {
            reasons.add("Android version $detectedVersion != ${PatchEngine.ANDROID_EXPECTED_VERSION}")
        }
        val modCnt = targets.count { it.state == "modified" && !it.hasBackup }
        val missCnt = targets.count { it.state == "missing" }
        // Hotfix-proof Smart Version Sync: resource hotfixes (r3219596 / r3273851 / r3300000+) stay Synced automatically
        // whenever VStringPool & Static UI bundles are dynamically discovered and valid!
        if (missCnt > 0 && detectedResRev.isNotEmpty() && detectedResRev != PatchEngine.ANDROID_EXPECTED_RES_REV) {
            reasons.add("Resource revision r$detectedResRev != r${PatchEngine.ANDROID_EXPECTED_RES_REV}")
        }
        if (modCnt > 0) reasons.add("$modCnt bundle(s) differ from verified SHA-256 hashes")
        if (missCnt > 0) reasons.add("$missCnt bundle(s) missing (finish in-game download first)")

        return ScanSnapshot(
            accessMode = mode,
            shizukuInstalled = shizukuInstalled,
            shizukuRunning = shizukuRunning,
            shizukuGranted = shizukuGranted,
            gameInstalled = gameInstalled,
            runningPids = runningPids,
            detectedVersion = detectedVersion,
            expectedVersion = PatchEngine.ANDROID_EXPECTED_VERSION,
            detectedResRev = detectedResRev,
            expectedResRev = PatchEngine.ANDROID_EXPECTED_RES_REV,
            versionStatus = if (reasons.isEmpty()) "synced" else "mismatch",
            versionReasons = reasons,
            targets = targets,
            newsCacheBytes = newsCacheBytes,
            hasNewsBackup = hasNewsBackup,
            backupDirPath = backupDir.absolutePath,
        )
    }

    private fun ensureBackupV2(
        mode: AccessMode,
        fileName: String,
        remotePath: String,
        bakFile: File,
        lang: PatchLang,
        custom: CustomPack?,
        entry: BundleState,
        knownEn: Set<String>,
        knownId: Set<String>,
        label: String,
        log: (String, LogLevel) -> Unit,
    ): Pair<Boolean, String> {
        val curSha = remoteSha256(mode, remotePath)
            ?: error("$label not found on device. Finish in-game resource download first.")
        val up = curSha.uppercase()
        val stMatch = entry.fileName == fileName && entry.patchedSha.isNotEmpty() &&
            up.equals(entry.patchedSha, ignoreCase = true)

        if (!bakFile.isFile) {
            val isEn = knownEn.contains(up) || (stMatch && entry.lang == "patched_en")
            val isId = knownId.contains(up) || (stMatch && entry.lang == "patched_id")
            val isCustom = stMatch && entry.lang == "patched_custom"
            if (isEn || isId || isCustom) {
                if (custom != null) {
                    error("$label is already patched and original backup is missing. Restore Original JP first before applying a Custom Language Pack.")
                }
                if (stMatch && ((lang == PatchLang.EN && isEn) || (lang == PatchLang.ID && isId))) {
                    return Pair(true, entry.origSha)
                }
                error("$label is already patched and original backup is missing. Restore Original JP first.")
            }
            log("  Backing up original $label [$fileName]...", LogLevel.INFO)
            val tmpBak = File(backupDir, "${bakFile.name}.tmp")
            copyFromRemote(mode, remotePath, tmpBak)
            val pulledSha = CryptoLz4.sha256File(tmpBak)
            if (!pulledSha.equals(curSha, ignoreCase = true)) {
                tmpBak.delete()
                error("Backup SHA-256 mismatch for $label")
            }
            if (!tmpBak.renameTo(bakFile)) {
                tmpBak.copyTo(bakFile, overwrite = true)
                tmpBak.delete()
            }
            log("  Saved verified original backup: ${bakFile.name} (SHA256=${pulledSha.take(12)}...)", LogLevel.OK)
            return Pair(false, pulledSha)
        }

        val origSha = if (entry.fileName == fileName && entry.origSha.isNotEmpty()) {
            entry.origSha
        } else {
            CryptoLz4.sha256File(bakFile)
        }
        if (custom != null) return Pair(false, origSha)
        val wantTag = if (lang == PatchLang.ID) "patched_id" else "patched_en"
        return Pair(stMatch && entry.lang == wantTag, origSha)
    }

    fun applyPatch(
        lang: PatchLang,
        custom: CustomPack? = null,
        log: (String, LogLevel) -> Unit,
        onProgress: (Int, Int, String) -> Unit,
    ): Int {
        val mode = resolveAccessMode()
        if (mode == AccessMode.NONE) error("Storage access not authorized. Start & authorize Shizuku or pick SAF folder first.")
        val pids = findRunningGamePids(mode)
        if (pids.isNotEmpty()) {
            error("Blocked: Game is currently running (PID ${pids.joinToString(", ")}). Force-close the game before patching!")
        }

        val stateFile = loadPatchState()
        val activeLabel = custom?.let { "${it.meta.name} (${it.meta.code} · base=${it.meta.base.uppercase()})" } ?: lang.label
        val activeCode = custom?.meta?.code ?: lang.shortCode

        var rewritten = 0
        log("=== Starting $activeLabel Patch (${mode.label}) ===", LogLevel.INFO)
        if (custom != null) {
            log("  Custom Pack: ${custom.meta.fileName} (${custom.meta.vstringCount} vstring + ${custom.meta.staticUiCount} static_ui overrides)", LogLevel.INFO)
        }
        onProgress(0, PatchEngine.ANDROID_TOTAL_STRINGS, "Discovering active Android bundles...")

        val discVs = discoverActiveVString(mode)
            ?: error("VStringPool bundle not found on device. Finish in-game resource download first.")
        val discUi = discoverActiveStaticUi(mode, stateFile)
            ?: error("Static UI bundle not found on device. Finish in-game resource download first.")

        // [1/2] VStringPool (universal hash-keyed!)
        log("=== [1/2] Patching VStringPool [${discVs.fileName} · $activeCode] ===", LogLevel.INFO)
        val vsRemote = "${PatchEngine.REMOTE_RAW_BUNDLE_DIR}/${discVs.fileName}"
        val vsBak = File(backupDir, "${discVs.fileName}.original")
        onProgress(0, PatchEngine.ANDROID_TOTAL_STRINGS, "Checking / backing up VStringPool...")
        val (vsUpToDate, vsOrigSha) = ensureBackupV2(
            mode, discVs.fileName, vsRemote, vsBak, lang, custom, stateFile.vstring,
            PatchEngine.ANDROID_VSTRING_KNOWN_EN_SHAS, PatchEngine.ANDROID_VSTRING_KNOWN_ID_SHAS,
            "VStringPool", log,
        )
        if (vsUpToDate) {
            val cnt = maxOf(stateFile.vstring.matchedCount, PatchEngine.VSTRING_STRINGS)
            log("  VStringPool [${discVs.fileName}] already matches $activeCode ($cnt strings).", LogLevel.OK)
            rewritten += cnt
            onProgress(rewritten, PatchEngine.ANDROID_TOTAL_STRINGS, "VStringPool already up-to-date")
        } else {
            val buf = vsBak.readBytes()
            val cnt = PatchEngine.patchAndroidVStringInPlace(buf, loader, lang, log, custom) { delta, stg ->
                onProgress(delta, PatchEngine.ANDROID_TOTAL_STRINGS, stg)
            }
            val staged = File(stageDir, "${discVs.fileName}.staged")
            staged.writeBytes(buf)
            val localSha = CryptoLz4.sha256File(staged)
            onProgress(cnt, PatchEngine.ANDROID_TOTAL_STRINGS, "Writing & verifying VStringPool...")
            val finalSha = try {
                pushToRemoteVerified(mode, staged, vsRemote, localSha)
            } finally {
                staged.delete()
            }
            stateFile.vstring = BundleState(
                fileName = discVs.fileName,
                subOffset = 0,
                lang = stateTagFor(lang, custom),
                customCode = custom?.meta?.code.orEmpty(),
                origSha = vsOrigSha,
                patchedSha = finalSha,
                matchedCount = cnt,
            )
            savePatchState(stateFile)
            log("  VStringPool [${discVs.fileName} · $activeCode] installed & verified! Rewritten=$cnt, SHA256=$finalSha", LogLevel.OK)
            rewritten += cnt
        }

        // [2/2] Static UI KeyConfig (dynamically discovered!)
        log("=== [2/2] Patching Static UI [${discUi.fileName} · $activeCode] ===", LogLevel.INFO)
        val uiRemote = "${PatchEngine.REMOTE_DEFAULT_BUNDLE_DIR}/${discUi.fileName}"
        val uiBak = File(backupDir, "${discUi.fileName}.original")
        onProgress(rewritten, PatchEngine.ANDROID_TOTAL_STRINGS, "Checking / backing up Static UI...")
        val (uiUpToDate, uiOrigSha) = ensureBackupV2(
            mode, discUi.fileName, uiRemote, uiBak, lang, custom, stateFile.staticUi,
            PatchEngine.ANDROID_STATIC_UI_KNOWN_EN_SHAS, PatchEngine.ANDROID_STATIC_UI_KNOWN_ID_SHAS,
            "Static UI KeyConfig", log,
        )
        if (uiUpToDate) {
            log("  Static UI [${discUi.fileName}] already matches $activeCode (${PatchEngine.STATIC_UI_STRINGS} strings).", LogLevel.OK)
            rewritten += PatchEngine.STATIC_UI_STRINGS
            onProgress(rewritten, PatchEngine.ANDROID_TOTAL_STRINGS, "Static UI already up-to-date")
        } else {
            val origBytes = uiBak.readBytes()
            val subOff = CryptoLz4.findStaticUiSubBundle(origBytes)?.first ?: discUi.subOffset
            val (outBytes, cnt) = PatchEngine.patchAndroidStaticUiBytes(origBytes, subOff, loader, lang, custom) { delta, stg ->
                onProgress(rewritten + delta, PatchEngine.ANDROID_TOTAL_STRINGS, stg)
            }
            val staged = File(stageDir, "${discUi.fileName}.staged")
            staged.writeBytes(outBytes)
            val localSha = CryptoLz4.sha256File(staged)
            onProgress(rewritten + cnt, PatchEngine.ANDROID_TOTAL_STRINGS, "Writing & verifying Static UI...")
            val finalSha = try {
                pushToRemoteVerified(mode, staged, uiRemote, localSha)
            } finally {
                staged.delete()
            }
            stateFile.staticUi = BundleState(
                fileName = discUi.fileName,
                subOffset = subOff,
                lang = stateTagFor(lang, custom),
                customCode = custom?.meta?.code.orEmpty(),
                origSha = uiOrigSha,
                patchedSha = finalSha,
                matchedCount = cnt,
            )
            savePatchState(stateFile)
            log("  Static UI [${discUi.fileName} · $activeCode] installed & verified! Rewritten=$cnt, SHA256=$finalSha", LogLevel.OK)
            rewritten += cnt
        }

        onProgress(rewritten, maxOf(rewritten, PatchEngine.ANDROID_TOTAL_STRINGS), "All targets verified 100% ($activeCode)!")
        log("=== COMPLETE [$activeLabel]! Rewritten & verified: $rewritten strings ===", LogLevel.OK)
        return rewritten
    }

    fun applyNewsPatch(
        lang: PatchLang,
        log: (String, LogLevel) -> Unit,
        onProgress: (Int, Int, String) -> Unit,
    ): Pair<Int, Int> {
        val mode = resolveAccessMode()
        if (mode == AccessMode.NONE) error("Storage access not authorized.")
        val pids = findRunningGamePids(mode)
        if (pids.isNotEmpty()) {
            error("Blocked: Game is currently running (PID ${pids.joinToString(", ")}). Force-close it first!")
        }

        val remoteLib = "${PatchEngine.REMOTE_FILES_ROOT}/Library"
        val remoteHc = "${PatchEngine.REMOTE_FILES_ROOT}/HTTPCache"
        val libSz = remoteSize(mode, remoteLib)
        if (libSz <= 16L) {
            error("News cache is empty. Open the in-game News/Announcements screen once, close the game, then click Translate News again.")
        }

        val newsWork = File(backupDir, "NewsWork").apply { mkdirs() }
        val localHc = File(newsWork, "HTTPCache").apply { mkdirs() }
        onProgress(0, 16, "Reading HTTP News cache...")
        copyFromRemote(mode, remoteLib, File(newsWork, "Library"))
        for (name in remoteListDir(mode, remoteHc)) {
            if (name.all { it.isLetterOrDigit() }) {
                try {
                    copyFromRemote(mode, "$remoteHc/$name", File(localHc, name))
                } catch (_: Throwable) {}
            }
        }

        val (patched, total) = PatchEngine.patchNewsCacheDir(newsWork, loader, lang, log, onProgress)
        onProgress(patched, maxOf(1, total), "Writing translated News cache...")
        val libFile = File(newsWork, "Library")
        pushToRemoteVerified(mode, libFile, remoteLib, CryptoLz4.sha256File(libFile))
        localHc.listFiles()?.forEach { f ->
            if (f.isFile) {
                pushToRemoteVerified(mode, f, "$remoteHc/${f.name}", CryptoLz4.sha256File(f))
            }
        }
        log("=== NEWS COMPLETE [${lang.shortCode}]! Translated $patched/$total announcements ===", LogLevel.OK)
        return Pair(patched, total)
    }

    fun rollbackToOriginal(
        log: (String, LogLevel) -> Unit,
        onProgress: (Int, Int, String) -> Unit,
    ): Int {
        val mode = resolveAccessMode()
        if (mode == AccessMode.NONE) error("Storage access not authorized.")
        val pids = findRunningGamePids(mode)
        if (pids.isNotEmpty()) {
            error("Blocked: Game is currently running (PID ${pids.joinToString(", ")}). Force-close it before rollback!")
        }

        val stateFile = loadPatchState()
        val discVs = discoverActiveVString(mode)
        val discUi = discoverActiveStaticUi(mode, stateFile)
        val vsName = discVs?.fileName ?: stateFile.vstring.fileName.ifBlank { "1qnamndresrohzjhuvt2lw.r" }
        val uiName = discUi?.fileName ?: stateFile.staticUi.fileName.ifBlank { "nibdvaxcnfh3en5bnvhtw.r" }

        data class RbSpec(val id: String, val name: String, val remote: String, val bakName: String)
        val specs = listOf(
            RbSpec("vstring", "VStringPool", "${PatchEngine.REMOTE_RAW_BUNDLE_DIR}/$vsName", "$vsName.original"),
            RbSpec("static_ui", "Static UI KeyConfig", "${PatchEngine.REMOTE_DEFAULT_BUNDLE_DIR}/$uiName", "$uiName.original"),
        )
        var restored = 0
        log("=== Starting Rollback to Original JP ===", LogLevel.WARN)
        for ((i, spec) in specs.withIndex()) {
            onProgress(i, specs.size, "Restoring ${spec.name}...")
            val bak = File(backupDir, spec.bakName)
            if (!bak.isFile) {
                log("  [SKIP] No backup found for ${spec.name} (${spec.bakName})", LogLevel.WARN)
                continue
            }
            val bakSha = CryptoLz4.sha256File(bak)
            val curSha = remoteSha256(mode, spec.remote).orEmpty()
            if (!curSha.equals(bakSha, ignoreCase = true)) {
                val postSha = pushToRemoteVerified(mode, bak, spec.remote, bakSha)
                log("  [RESTORED] ${spec.name} -> Original JP (SHA256=${postSha.take(16)}...)", LogLevel.OK)
            } else {
                log("  [OK] ${spec.name} is already Original JP.", LogLevel.OK)
            }
            val st = if (spec.id == "vstring") stateFile.vstring else stateFile.staticUi
            st.lang = "original"
            st.customCode = ""
            st.patchedSha = ""
            st.origSha = bakSha
            restored++
        }
        savePatchState(stateFile)

        val newsWork = File(backupDir, "NewsWork")
        if (File(newsWork, "_NewsPatchBackup/Library").isFile) {
            val n = PatchEngine.rollbackNewsCacheDir(newsWork, log)
            if (n > 0) {
                val remoteLib = "${PatchEngine.REMOTE_FILES_ROOT}/Library"
                val remoteHc = "${PatchEngine.REMOTE_FILES_ROOT}/HTTPCache"
                val libFile = File(newsWork, "Library")
                pushToRemoteVerified(mode, libFile, remoteLib, CryptoLz4.sha256File(libFile))
                File(newsWork, "HTTPCache").listFiles()?.forEach { f ->
                    if (f.isFile) pushToRemoteVerified(mode, f, "$remoteHc/${f.name}", CryptoLz4.sha256File(f))
                }
                log("  [RESTORED] Cached News ($n announcements) -> Original JP", LogLevel.OK)
            }
        }

        onProgress(specs.size, specs.size, "Rollback complete ($restored/${specs.size} restored).")
        log("=== Rollback complete! Restored $restored/${specs.size} targets ===", LogLevel.OK)
        return restored
    }
}
