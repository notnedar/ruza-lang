package com.ruzalang.patcher.storage

import androidx.annotation.Keep
import com.ruzalang.patcher.IPatcherFileService
import com.ruzalang.patcher.engine.CryptoLz4
import java.io.BufferedReader
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.InputStreamReader
import kotlin.system.exitProcess

/**
 * Runs inside Shizuku's privileged shell process (uid=2000, group ext_data_rw).
 * Performs fast on-disk hashing, atomic file copy, directory listing, and pidof checks
 * without sending multi-megabyte files over Binder IPC.
 */
@Keep
class PatcherFileService : IPatcherFileService.Stub() {

    override fun destroy() {
        exitProcess(0)
    }

    override fun fileExists(path: String): Boolean = File(path).isFile

    override fun fileSize(path: String): Long {
        val f = File(path)
        return if (f.isFile) f.length() else -1L
    }

    override fun sha256File(path: String): String {
        val f = File(path)
        if (!f.isFile) return ""
        return try {
            CryptoLz4.sha256File(f)
        } catch (e: Throwable) {
            "ERR:${e.message}"
        }
    }

    override fun listDir(path: String): String {
        val d = File(path)
        if (!d.isDirectory) return ""
        val names = d.list() ?: return ""
        return names.sorted().joinToString("\n")
    }

    override fun copyFileAtomic(srcPath: String, dstPath: String): String {
        return try {
            val src = File(srcPath)
            val dst = File(dstPath)
            if (!src.isFile) return "ERR:Source not found: $srcPath"
            dst.parentFile?.mkdirs()
            val expectedSha = CryptoLz4.sha256File(src)
            val tmp = File("${dst.absolutePath}.ruza_tmp")
            val buf = ByteArray(4 * 1024 * 1024)
            FileInputStream(src).use { ins ->
                FileOutputStream(tmp).use { out ->
                    while (true) {
                        val n = ins.read(buf)
                        if (n <= 0) break
                        out.write(buf, 0, n)
                    }
                    out.fd.sync()
                }
            }
            val tmpSha = CryptoLz4.sha256File(tmp)
            if (!tmpSha.equals(expectedSha, ignoreCase = true)) {
                tmp.delete()
                return "ERR:Transfer SHA-256 mismatch ($tmpSha != $expectedSha)"
            }
            if (dst.exists() && !tmp.renameTo(dst)) {
                // Fallback if cross-mountpoint (e.g. /data -> /sdcard): overwrite directly after verification
                FileInputStream(tmp).use { ins ->
                    FileOutputStream(dst).use { out ->
                        while (true) {
                            val n = ins.read(buf)
                            if (n <= 0) break
                            out.write(buf, 0, n)
                        }
                        out.fd.sync()
                    }
                }
                tmp.delete()
            } else if (!dst.exists() && !tmp.renameTo(dst)) {
                tmp.delete()
                return "ERR:Atomic rename failed for $dstPath"
            }
            val finalSha = CryptoLz4.sha256File(dst)
            if (!finalSha.equals(expectedSha, ignoreCase = true)) {
                return "ERR:Post-move SHA-256 mismatch ($finalSha != $expectedSha)"
            }
            "OK:$finalSha"
        } catch (e: Throwable) {
            "ERR:${e.message}"
        }
    }

    private fun runCmd(vararg cmd: String): String {
        return try {
            val p = ProcessBuilder(*cmd).redirectErrorStream(true).start()
            val sb = StringBuilder()
            BufferedReader(InputStreamReader(p.inputStream)).use { br ->
                while (true) {
                    val line = br.readLine() ?: break
                    sb.append(line).append('\n')
                }
            }
            p.waitFor()
            sb.toString()
        } catch (_: Throwable) {
            ""
        }
    }

    override fun findRunningPids(packageName: String): String {
        return runCmd("pidof", packageName).trim()
    }

    override fun getPackageVersion(packageName: String): String {
        val out = runCmd("dumpsys", "package", packageName)
        for (line in out.lineSequence()) {
            val idx = line.indexOf("versionName=")
            if (idx >= 0) {
                return line.substring(idx + 12).trim().split(Regex("\\s+")).firstOrNull().orEmpty()
            }
        }
        return ""
    }

    override fun isPackageInstalled(packageName: String): Boolean {
        val out = runCmd("pm", "path", packageName)
        return out.contains("package:")
    }
}
