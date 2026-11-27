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

        assertTrue(UpdateChecker.isNewer("v0.9.0", "0.8.1"))
        assertTrue(!UpdateChecker.isNewer("v0.9.0", "0.9.0"))
        assertTrue(!UpdateChecker.isNewer("v0.8.1", "0.9.0"))
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
                        "3216E4E2A56F2BC36EAA2FFF79B203C854FA834A93A97C2CA218B40E909FF5F1",
                        "2A4428AE21009DB19621A123B1A2561ED1F83A685B4C9160B94BA74CD50E8DAC",
                    ),
                    Triple(
                        PatchLang.ID,
                        "EBC5EE0155910A58D3B62AB978A886CCD1BA4FA68F4CE929B7763E32454531DE",
                        "60F0B86F98FCA84D46AE1AD7531510320812972D5371F80FC358860B660BC308",
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
                assertNotEquals("3216E4E2A56F2BC36EAA2FFF79B203C854FA834A93A97C2CA218B40E909FF5F1", CryptoLz4.sha256Bytes(vsBuf))

                val (uiOut, uiCnt) = PatchEngine.patchAndroidStaticUiBytes(uiOrig, foundUi.first, loader, PatchLang.EN, esPack) { _, _ -> }
                assertEquals(3135, uiCnt)
                assertEquals(uiOrig.size, uiOut.size)
                assertNotEquals("2A4428AE21009DB19621A123B1A2561ED1F83A685B4C9160B94BA74CD50E8DAC", CryptoLz4.sha256Bytes(uiOut))
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
                        "9C4283483E66F334929C92E620BB1A7345E83CBF10DF29DF4BD17CBB20687F05",
                        "B0CF69FFF31ACB0D3D9D6B9A267D0C16B15A64FBE9E27E9FA93CF7468DDDF11C",
                    ),
                    Triple(
                        PatchLang.ID,
                        "5B0C27CBBA77FD7B9BCAC0D4A263177FCAEFE7FAEBB20A1516BA802028183D4A",
                        "6A64B6FCEAA1C42264F966A1870F4A36E8B1044D93EF1DFDCA046F0617E7FE5F",
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
