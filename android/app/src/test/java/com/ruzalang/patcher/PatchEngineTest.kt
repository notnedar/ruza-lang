package com.ruzalang.patcher

import com.ruzalang.patcher.engine.CryptoLz4
import com.ruzalang.patcher.engine.PatchEngine
import com.ruzalang.patcher.engine.PatchLang
import com.ruzalang.patcher.engine.PayloadLoader
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

class PatchEngineTest {

    private val repoResources = listOf(File("../../resources"), File("../resources"))
        .firstOrNull { it.isDirectory }
        ?: error("resources dir not found from ${File(".").absolutePath}")
    private val loader = object : PayloadLoader {
        override fun load(name: String): ByteArray = File(repoResources, name).readBytes()
    }

    @Test
    fun testLz4ExactRoundTripAndSemver() {
        val sample = ByteArray(16384) { i -> ((i * 7 + (i / 17)) and 0xFF).toByte() }
        val minComp = CryptoLz4.lz4Compress(sample, 256)
        assertTrue(CryptoLz4.lz4Decompress(minComp, sample.size).contentEquals(sample))
        val target = minComp.size + 137
        val exact = CryptoLz4.lz4CompressExact(sample, target, 256)
        assertEquals(target, exact.size)
        assertTrue(CryptoLz4.lz4Decompress(exact, sample.size).contentEquals(sample))

        assertTrue(UpdateChecker.isNewer("v0.7.1", "0.7.0"))
        assertTrue(!UpdateChecker.isNewer("v0.7.0", "0.7.0"))
        assertTrue(!UpdateChecker.isNewer("v0.6.1", "0.7.0"))
    }

    @Test
    fun testAndroidRemapTableCount() {
        val enMap = PatchEngine.loadAndroidVStringMap(loader, PatchLang.EN)
        val idMap = PatchEngine.loadAndroidVStringMap(loader, PatchLang.ID)
        assertEquals(74055, enMap.size)
        assertEquals(74055, idMap.size)
    }

    @Test
    fun testBitIdenticalAndroidBundlesIfLocalBackupPresent() {
        val la = System.getenv("LOCALAPPDATA") ?: return
        val bakDir = File(la, "RuzaLang/AndroidBackups/emulator-5554")
        val vsOrigFile = File(bakDir, "1qnamndresrohzjhuvt2lw.r.original")
        val uiOrigFile = File(bakDir, "nibdvaxcnfh3en5bnvhtw.r.original")
        if (!vsOrigFile.isFile || !uiOrigFile.isFile) return

        // Verify EN & ID produce 100% BIT-FOR-BIT IDENTICAL SHA-256 to Rust engine!
        for ((lang, expVs, expUi) in listOf(
            Triple(PatchLang.EN, PatchEngine.ANDROID_VSTRING_EN_SHA, PatchEngine.ANDROID_STATIC_UI_EN_SHA),
            Triple(PatchLang.ID, PatchEngine.ANDROID_VSTRING_ID_SHA, PatchEngine.ANDROID_STATIC_UI_ID_SHA),
        )) {
            val vsBuf = vsOrigFile.readBytes()
            val vsCnt = PatchEngine.patchAndroidVStringInPlace(vsBuf, loader, lang, { _, _ -> }, { _, _ -> })
            assertEquals(74055, vsCnt)
            assertEquals(expVs, CryptoLz4.sha256Bytes(vsBuf))

            val uiOrig = uiOrigFile.readBytes()
            val (uiOut, uiCnt) = PatchEngine.patchAndroidStaticUiBytes(uiOrig, loader, lang) { _, _ -> }
            assertEquals(3135, uiCnt)
            assertEquals(uiOrig.size, uiOut.size)
            assertEquals(expUi, CryptoLz4.sha256Bytes(uiOut))
        }
    }
}
