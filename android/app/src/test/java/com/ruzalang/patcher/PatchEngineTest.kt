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

        assertTrue(UpdateChecker.isNewer("v0.9.2", "0.9.1"))
        assertTrue(!UpdateChecker.isNewer("v0.9.2", "0.9.2"))
        assertTrue(!UpdateChecker.isNewer("v0.9.1", "0.9.2"))
    }

    @Test
    fun testCustomPackParsingAndRzv2HashTable() {
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

        // JP base only maps the provided vstring IDs via RZV2 pcId -> (fnv1a64, jpLen)
        val jpOnly = """{ "meta": { "name": "Minimal", "code": "MIN", "base": "jp" }, "vstring": { "1504449": "Equipar" } }"""
        val p3 = PatchEngine.parseCustomPackJson(jpOnly, "min.json")
        val hashMap = PatchEngine.buildVStringHashTable(loader, PatchLang.EN, p3)
        assertEquals(1, hashMap.size)
    }

    @Test
    fun testUniversalRzv2TableCount() {
        val enMap = PatchEngine.buildVStringHashTable(loader, PatchLang.EN)
        val idMap = PatchEngine.buildVStringHashTable(loader, PatchLang.ID)
        assertEquals(76180, enMap.size)
        assertEquals(76180, idMap.size)
    }

    @Test
    fun testBitIdenticalCrossBuildAgainstRustEngineIfLocalBackupsPresent() {
        // 1. Android r3219596 backup -> MUST be 100% BIT-FOR-BIT IDENTICAL to Rust SHA-256!
        val la = System.getenv("LOCALAPPDATA")
        if (la != null) {
            val bakDir = File(la, "RuzaLang/AndroidBackups/emulator-5554")
            val vsOrigFile = File(bakDir, "1qnamndresrohzjhuvt2lw.r.original")
            val uiOrigFile = File(bakDir, "nibdvaxcnfh3en5bnvhtw.r.original")
            if (vsOrigFile.isFile && uiOrigFile.isFile) {
                val uiOrig = uiOrigFile.readBytes()
                val foundUi = CryptoLz4.findStaticUiSubBundle(uiOrig) ?: error("Static UI sub-bundle not discovered")
                assertEquals(4200075, foundUi.first)
                assertEquals(83151, foundUi.second)

                for ((lang, expVs, expUi) in listOf(
                    Triple(
                        PatchLang.EN,
                        "08DC13585E76D90C11784B3B5F17E5A829FC353FFADE19A744FF9936477EBBBF",
                        "EB18599A3513CDF558F4716E0B70DEA233CAC9076D50BA6EFEB0D7958C87F01B",
                    ),
                    Triple(
                        PatchLang.ID,
                        "42B324F18C05D8F4DFEFCA605F82D082526CB54C1F621BE77991CD3F13EE305D",
                        "10939AD972515B9A6DF118AC5D7ADA99BBA7F0C0FBF54FCAE79698987B7EE59B",
                    ),
                )) {
                    val vsBuf = vsOrigFile.readBytes()
                    val vsCnt = PatchEngine.patchAndroidVStringInPlace(vsBuf, loader, lang, { _, _ -> }) { _, _ -> }
                    assertEquals(74058, vsCnt)
                    assertEquals(expVs, CryptoLz4.sha256Bytes(vsBuf))

                    val (uiOut, uiCnt) = PatchEngine.patchAndroidStaticUiBytes(uiOrig, foundUi.first, loader, lang) { _, _ -> }
                    assertEquals(3135, uiCnt)
                    assertEquals(uiOrig.size, uiOut.size)
                    assertEquals(expUi, CryptoLz4.sha256Bytes(uiOut))
                }

                // Custom Language Pack overlay
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
                assertEquals(74058, vsCnt)
                assertNotEquals("08DC13585E76D90C11784B3B5F17E5A829FC353FFADE19A744FF9936477EBBBF", CryptoLz4.sha256Bytes(vsBuf))

                val (uiOut, uiCnt) = PatchEngine.patchAndroidStaticUiBytes(uiOrig, foundUi.first, loader, PatchLang.EN, esPack) { _, _ -> }
                assertEquals(3135, uiCnt)
                assertEquals(uiOrig.size, uiOut.size)
                assertNotEquals("EB18599A3513CDF558F4716E0B70DEA233CAC9076D50BA6EFEB0D7958C87F01B", CryptoLz4.sha256Bytes(uiOut))
            }
        }

        // 2. Hotfix r3273853 backup -> MUST also be 100% BIT-FOR-BIT IDENTICAL to Rust SHA-256!
        val r3273Dir = listOf(File("../../../backups/r3273853"), File("../../backups/r3273853")).firstOrNull { it.isDirectory }
        if (r3273Dir != null) {
            val vs3273 = File(r3273Dir, "heqmts9lcjdvbupu0x2wpg.r.original")
            val ui3273 = File(r3273Dir, "h3y7ujukwn9qbhoqhzkkg.r.original")
            if (vs3273.isFile && ui3273.isFile) {
                assertEquals(1667303, PatchEngine.probeVStringFile(vs3273, loader.load("mmap_key_4096.bin")))
                val uiOrig = ui3273.readBytes()
                val foundUi = CryptoLz4.findStaticUiSubBundle(uiOrig) ?: error("r3273853 Static UI not discovered")
                assertEquals(420062, foundUi.first)
                assertEquals(83081, foundUi.second)

                for ((lang, expVs, expUi) in listOf(
                    Triple(
                        PatchLang.EN,
                        "2C9AB1152E21BFDC05B8C218EC8F81B4947B31CDE99DE67948D51A94B5899D40",
                        "7900C693A5C922A52B376AB3B62D03DA18E5C03E446B812D1D49CE3DA3FD6683",
                    ),
                    Triple(
                        PatchLang.ID,
                        "33A485100DC66FC37DAF9181E2EF6D7E7DCE83C564F5D23DC0B12BB6AEC4FAE8",
                        "DE9E6053428625B8CC78E4C126EB8BDDB41A4B5E5D895FD7FE14C5A37CE59336",
                    ),
                )) {
                    val vsBuf = vs3273.readBytes()
                    val vsCnt = PatchEngine.patchAndroidVStringInPlace(vsBuf, loader, lang, { _, _ -> }) { _, _ -> }
                    assertEquals(76180, vsCnt)
                    assertEquals(expVs, CryptoLz4.sha256Bytes(vsBuf))

                    val (uiOut, uiCnt) = PatchEngine.patchAndroidStaticUiBytes(uiOrig, foundUi.first, loader, lang) { _, _ -> }
                    assertEquals(3135, uiCnt)
                    assertEquals(uiOrig.size, uiOut.size)
                    assertEquals(expUi, CryptoLz4.sha256Bytes(uiOut))
                }
            }
        }
    }
}
