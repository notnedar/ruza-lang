package com.ruzalang.patcher.storage

import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import androidx.documentfile.provider.DocumentFile
import com.ruzalang.patcher.IPatcherFileService
import com.ruzalang.patcher.engine.CryptoLz4
import com.ruzalang.patcher.engine.CustomPack
import com.ruzalang.patcher.engine.LogLevel
import com.ruzalang.patcher.engine.PatchEngine
import com.ruzalang.patcher.engine.PatchLang
import com.ruzalang.patcher.engine.PayloadLoader
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
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

    private fun isDirectFsWritable(): Boolean {
        return try {
            val root = File(PatchEngine.REMOTE_FILES_ROOT)
            val vs = File(PatchEngine.REMOTE_VSTRING)
            (vs.isFile && vs.canRead() && vs.canWrite()) || (root.isDirectory && root.canRead() && root.canWrite())
        } catch (_: Throwable) {
            false
        }
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

        data class Spec(
            val id: String,
            val name: String,
            val sub: String,
            val remote: String,
            val bakName: String,
            val count: Int,
            val origSha: String,
            val enSha: String,
            val idSha: String,
        )

        val specs = listOf(
            Spec(
                "vstring", "VStringPool", "Story, Quests, Skills, Kibo, Settings",
                PatchEngine.REMOTE_VSTRING, "1qnamndresrohzjhuvt2lw.r.original", 74055,
                PatchEngine.ANDROID_VSTRING_ORIG_SHA, PatchEngine.ANDROID_VSTRING_EN_SHA, PatchEngine.ANDROID_VSTRING_ID_SHA,
            ),
            Spec(
                "static_ui", "Static UI KeyConfig", "Prefab Buttons, Headers, Notices",
                PatchEngine.REMOTE_STATIC_UI, "nibdvaxcnfh3en5bnvhtw.r.original", 3135,
                PatchEngine.ANDROID_STATIC_UI_ORIG_SHA, PatchEngine.ANDROID_STATIC_UI_EN_SHA, PatchEngine.ANDROID_STATIC_UI_ID_SHA,
            ),
        )

        val targets = ArrayList<AndroidTargetStatus>(2)
        for (s in specs) {
            val hasBak = File(backupDir, s.bakName).isFile
            val sha = remoteSha256(mode, s.remote)
            if (sha == null) {
                log("  [MISSING] ${s.name}", LogLevel.WARN)
                targets.add(AndroidTargetStatus(s.id, s.name, s.sub, s.count, "missing", hasBak, ""))
                continue
            }
            val state = when {
                sha.equals(s.idSha, ignoreCase = true) -> "patched_id"
                sha.equals(s.enSha, ignoreCase = true) -> "patched_en"
                sha.equals(s.origSha, ignoreCase = true) -> "original"
                else -> "modified"
            }
            val lvl = if (state == "patched_en" || state == "patched_id") LogLevel.OK else LogLevel.INFO
            val lbl = when (state) {
                "patched_en" -> "ENGLISH"
                "patched_id" -> "INDONESIAN"
                "original" -> "ORIGINAL JP"
                else -> "MODIFIED"
            }
            log("  [$lbl] ${s.name} (${s.count} strings, backup=${if (hasBak) "YES" else "NO"})", lvl)
            targets.add(AndroidTargetStatus(s.id, s.name, s.sub, s.count, state, hasBak, sha))
        }

        val reasons = mutableListOf<String>()
        if (detectedVersion.isNotEmpty() && detectedVersion != PatchEngine.ANDROID_EXPECTED_VERSION) {
            reasons.add("Android version $detectedVersion != ${PatchEngine.ANDROID_EXPECTED_VERSION}")
        }
        if (detectedResRev.isNotEmpty() && detectedResRev != PatchEngine.ANDROID_EXPECTED_RES_REV) {
            reasons.add("Resource revision r$detectedResRev != r${PatchEngine.ANDROID_EXPECTED_RES_REV}")
        }
        val modCnt = targets.count { it.state == "modified" && !it.hasBackup }
        val missCnt = targets.count { it.state == "missing" }
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

    private fun ensureBackup(
        mode: AccessMode,
        remotePath: String,
        bakFile: File,
        lang: PatchLang,
        custom: CustomPack?,
        enSha: String,
        idSha: String,
        label: String,
        log: (String, LogLevel) -> Unit,
    ): Boolean {
        val curSha = remoteSha256(mode, remotePath)
            ?: error("$label not found on device. Finish in-game resource download first.")
        if (!bakFile.isFile) {
            val isEn = curSha.equals(enSha, ignoreCase = true)
            val isId = curSha.equals(idSha, ignoreCase = true)
            if (isEn || isId) {
                if (custom != null) {
                    error("$label is already patched and original backup is missing. Restore Original JP first before applying a Custom Language Pack.")
                }
                if (lang == PatchLang.EN && isEn) return true
                if (lang == PatchLang.ID && isId) return true
                error("$label is already patched and original backup is missing. Restore Original JP first.")
            }
            log("  Backing up original $label...", LogLevel.INFO)
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
            return false
        }
        if (custom != null) return false
        val wantSha = if (lang == PatchLang.ID) idSha else enSha
        return curSha.equals(wantSha, ignoreCase = true)
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

        val activeLabel = custom?.let { "${it.meta.name} (${it.meta.code} · base=${it.meta.base.uppercase()})" } ?: lang.label
        val activeCode = custom?.meta?.code ?: lang.shortCode

        var rewritten = 0
        log("=== Starting $activeLabel Patch (${mode.label}) ===", LogLevel.INFO)
        if (custom != null) {
            log("  Custom Pack: ${custom.meta.fileName} (${custom.meta.vstringCount} vstring + ${custom.meta.staticUiCount} static_ui overrides)", LogLevel.INFO)
        }
        onProgress(0, PatchEngine.ANDROID_TOTAL_STRINGS, "Checking Android bundles...")

        // [1/2] VStringPool (74,055 strings)
        log("=== [1/2] Patching VStringPool ($activeCode) ===", LogLevel.INFO)
        val vsBak = File(backupDir, "1qnamndresrohzjhuvt2lw.r.original")
        onProgress(0, PatchEngine.ANDROID_TOTAL_STRINGS, "Checking / backing up VStringPool (130 MB)...")
        val vsUpToDate = ensureBackup(
            mode, PatchEngine.REMOTE_VSTRING, vsBak, lang, custom,
            PatchEngine.ANDROID_VSTRING_EN_SHA, PatchEngine.ANDROID_VSTRING_ID_SHA, "VStringPool", log,
        )
        if (vsUpToDate) {
            log("  VStringPool already matches $activeCode (74,055 strings).", LogLevel.OK)
            rewritten += 74055
            onProgress(rewritten, PatchEngine.ANDROID_TOTAL_STRINGS, "VStringPool already up-to-date")
        } else {
            val buf = vsBak.readBytes()
            val cnt = PatchEngine.patchAndroidVStringInPlace(buf, loader, lang, log, custom) { delta, stg ->
                onProgress(delta, PatchEngine.ANDROID_TOTAL_STRINGS, stg)
            }
            val staged = File(stageDir, "1qnamndresrohzjhuvt2lw.r.staged")
            staged.writeBytes(buf)
            val localSha = CryptoLz4.sha256File(staged)
            onProgress(cnt, PatchEngine.ANDROID_TOTAL_STRINGS, "Writing & verifying VStringPool (130 MB)...")
            val finalSha = try {
                pushToRemoteVerified(mode, staged, PatchEngine.REMOTE_VSTRING, localSha)
            } finally {
                staged.delete()
            }
            log("  VStringPool [$activeCode] installed & verified! Rewritten=$cnt, SHA256=$finalSha", LogLevel.OK)
            rewritten += cnt
        }

        // [2/2] Static UI KeyConfig (3,135 strings)
        log("=== [2/2] Patching Static UI KeyConfig ($activeCode) ===", LogLevel.INFO)
        val uiBak = File(backupDir, "nibdvaxcnfh3en5bnvhtw.r.original")
        onProgress(rewritten, PatchEngine.ANDROID_TOTAL_STRINGS, "Checking / backing up Static UI (6 MB)...")
        val uiUpToDate = ensureBackup(
            mode, PatchEngine.REMOTE_STATIC_UI, uiBak, lang, custom,
            PatchEngine.ANDROID_STATIC_UI_EN_SHA, PatchEngine.ANDROID_STATIC_UI_ID_SHA, "Static UI KeyConfig", log,
        )
        if (uiUpToDate) {
            log("  Static UI KeyConfig already matches $activeCode (3,135 strings).", LogLevel.OK)
            rewritten += 3135
            onProgress(rewritten, PatchEngine.ANDROID_TOTAL_STRINGS, "Static UI already up-to-date")
        } else {
            val origBytes = uiBak.readBytes()
            val (outBytes, cnt) = PatchEngine.patchAndroidStaticUiBytes(origBytes, loader, lang, custom) { delta, stg ->
                onProgress(rewritten + delta, PatchEngine.ANDROID_TOTAL_STRINGS, stg)
            }
            val staged = File(stageDir, "nibdvaxcnfh3en5bnvhtw.r.staged")
            staged.writeBytes(outBytes)
            val localSha = CryptoLz4.sha256File(staged)
            onProgress(rewritten + cnt, PatchEngine.ANDROID_TOTAL_STRINGS, "Writing & verifying Static UI (6 MB)...")
            val finalSha = try {
                pushToRemoteVerified(mode, staged, PatchEngine.REMOTE_STATIC_UI, localSha)
            } finally {
                staged.delete()
            }
            log("  Static UI KeyConfig [$activeCode] installed & verified! Rewritten=$cnt, SHA256=$finalSha", LogLevel.OK)
            rewritten += cnt
        }

        onProgress(rewritten, PatchEngine.ANDROID_TOTAL_STRINGS, "All targets verified 100% ($activeCode)!")
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

        val specs = listOf(
            Triple("VStringPool", PatchEngine.REMOTE_VSTRING, "1qnamndresrohzjhuvt2lw.r.original"),
            Triple("Static UI KeyConfig", PatchEngine.REMOTE_STATIC_UI, "nibdvaxcnfh3en5bnvhtw.r.original"),
        )
        var restored = 0
        log("=== Starting Rollback to Original JP ===", LogLevel.WARN)
        for ((i, spec) in specs.withIndex()) {
            val (name, remote, bakName) = spec
            onProgress(i, specs.size, "Restoring $name...")
            val bak = File(backupDir, bakName)
            if (!bak.isFile) {
                log("  [SKIP] No backup found for $name ($bakName)", LogLevel.WARN)
                continue
            }
            val bakSha = CryptoLz4.sha256File(bak)
            val curSha = remoteSha256(mode, remote).orEmpty()
            if (curSha.equals(bakSha, ignoreCase = true)) {
                log("  [OK] $name is already Original JP.", LogLevel.OK)
                restored++
                continue
            }
            val postSha = pushToRemoteVerified(mode, bak, remote, bakSha)
            log("  [RESTORED] $name -> Original JP (SHA256=${postSha.take(16)}...)", LogLevel.OK)
            restored++
        }

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
