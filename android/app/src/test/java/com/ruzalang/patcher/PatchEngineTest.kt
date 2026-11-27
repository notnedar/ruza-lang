package com.ruzalang.patcher

import com.ruzalang.patcher.engine.CryptoLz4
import com.ruzalang.patcher.engine.CustomBase
import com.ruzalang.patcher.engine.PatchEngine
import com.ruzalang.patcher.engine.PatchLang
import com.ruzalang.patcher.engine.PayloadLoader
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
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

        assertTrue(UpdateChecker.isNewer("v0.8.0", "0.7.1"))
        assertTrue(!UpdateChecker.isNewer("v0.8.0", "0.8.0"))
        assertTrue(!UpdateChecker.isNewer("v0.7.1", "0.8.0"))
    }

    @Test
    fun testCustomPackParsingAndRemap() {
        val structured = """
            {
              "_comment": "demo",
              "meta": { "name": "Español (ES)", "code": "es", "author": "Tester", "base": "en" },
              "vstring": { "_note": "skip", "1504449": "Equipar", "1526899": "Bloqueo Perfecto" },
              "static_ui": { "15DC305A": "Equipar" },
              "builtin": { "confirm": "Confirmar" }
            }
        """.trimIndent()
        val p1 = PatchEngine.parseCustomPackJson(structured, "es-pack.json")
        assertEquals("ES", p1.meta.code)
        assertEquals(CustomBase.EN, p1.base)
        assertEquals(2, p1.meta.vstringCount)
        assertEquals(1, p1.meta.staticUiCount)
        assertEquals(1, p1.meta.builtinCount)

        // Flat dictionary drop-in (including all-digit 8-hex Static UI key!)
        val flat = """{ "_ignore": "x", "1504449": "Equipar", "15DC305A": "Equipar", "11369082": "Botón", "CB185163+1": "Confirmar" }"""
        val p2 = PatchEngine.parseCustomPackJson(flat, "dropin.json")
        assertEquals("dropin", p2.meta.name)
        assertEquals(1, p2.vstring.size)
        assertEquals(3, p2.staticUi.size)

        // JP base only maps the provided vstring IDs
        val jpOnly = """{ "meta": { "name": "Minimal", "code": "MIN", "base": "jp" }, "vstring": { "1504449": "Equipar" } }"""
        val p3 = PatchEngine.parseCustomPackJson(jpOnly, "min.json")
        val anMap = PatchEngine.loadAndroidVStringMap(loader, PatchLang.EN, p3)
        assertEquals(1, anMap.size)
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
            val vsCnt = PatchEngine.patchAndroidVStringInPlace(vsBuf, loader, lang, { _, _ -> }) { _, _ -> }
            assertEquals(74055, vsCnt)
            assertEquals(expVs, CryptoLz4.sha256Bytes(vsBuf))

            val uiOrig = uiOrigFile.readBytes()
            val (uiOut, uiCnt) = PatchEngine.patchAndroidStaticUiBytes(uiOrig, loader, lang) { _, _ -> }
            assertEquals(3135, uiCnt)
            assertEquals(uiOrig.size, uiOut.size)
            assertEquals(expUi, CryptoLz4.sha256Bytes(uiOut))
        }

        // Verify Custom Language Pack produces valid exact-size bundles with distinct SHA-256
        val esJson = """
            {
              "meta": { "name": "Español Test", "code": "ES", "author": "Test", "base": "en" },
              "vstring": { "1504449": "Equipar", "1526899": "Bloqueo Perfecto" },
              "static_ui": { "15DC305A": "Equipar" }
            }
        """.trimIndent()
        val esPack = PatchEngine.parseCustomPackJson(esJson, "es.json")
        val vsBuf = vsOrigFile.readBytes()
        val vsCnt = PatchEngine.patchAndroidVStringInPlace(vsBuf, loader, PatchLang.EN, { _, _ -> }, esPack) { _, _ -> }
        assertEquals(74055, vsCnt)
        assertNotEquals(PatchEngine.ANDROID_VSTRING_EN_SHA, CryptoLz4.sha256Bytes(vsBuf))

        val uiOrig = uiOrigFile.readBytes()
        val (uiOut, uiCnt) = PatchEngine.patchAndroidStaticUiBytes(uiOrig, loader, PatchLang.EN, esPack) { _, _ -> }
        assertEquals(3135, uiCnt)
        assertEquals(uiOrig.size, uiOut.size)
        assertNotEquals(PatchEngine.ANDROID_STATIC_UI_EN_SHA, CryptoLz4.sha256Bytes(uiOut))
    }
}
