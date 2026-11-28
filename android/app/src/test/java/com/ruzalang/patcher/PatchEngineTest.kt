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

        assertTrue(UpdateChecker.isNewer("v0.9.4", "0.9.3"))
        assertTrue(!UpdateChecker.isNewer("v0.9.4", "0.9.4"))
        assertTrue(!UpdateChecker.isNewer("v0.9.3", "0.9.4"))
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
                        "358A1F81F987304C8A9BB8E862F26045BFD779B160F7ED4D28CACDBEBE2EC15E",
                        "93A6E529D3773A08D98F40678F540C3D83B927E924295F8D75C24AF1A9CCEA6F",
                    ),
                    Triple(
                        PatchLang.ID,
                        "94C10742827E93879306B767055BE1B7D0B9F53DC8AEE18C32D049097EE374A7",
                        "505D56196E49B08C9B1FC79A40D06BC42655C21EA3BAF1DC48645C2A4C674C2E",
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
                assertNotEquals("358A1F81F987304C8A9BB8E862F26045BFD779B160F7ED4D28CACDBEBE2EC15E", CryptoLz4.sha256Bytes(vsBuf))

                val (uiOut, uiCnt) = PatchEngine.patchAndroidStaticUiBytes(uiOrig, foundUi.first, loader, PatchLang.EN, esPack) { _, _ -> }
                assertEquals(3135, uiCnt)
                assertEquals(uiOrig.size, uiOut.size)
                assertNotEquals("93A6E529D3773A08D98F40678F540C3D83B927E924295F8D75C24AF1A9CCEA6F", CryptoLz4.sha256Bytes(uiOut))
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
                        "3368C307936EA0C3E198FA1C9D094D4CFC021D580C59EF622B90AD08FB8DC73F",
                        "32E114D65E87D9B9B41E60566E191DC2B8C1AB534421B54CEB0684E0F257C576",
                    ),
                    Triple(
                        PatchLang.ID,
                        "207AE0CA54FF4017FAB2C3818069FA1353F36FF8978A9892800A62986EBF800C",
                        "9F609EA2D2EAF5E4FA23CA6DBA09FE6E98A6213B3D0FB46D20CAD7C39E46C9B5",
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
