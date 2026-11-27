package com.ruzalang.patcher

import android.annotation.SuppressLint
import android.content.ClipData
import android.content.ClipboardManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.content.pm.PackageManager
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.IBinder
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.provider.Settings
import android.webkit.JavascriptInterface
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebView
import android.widget.FrameLayout
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.webkit.WebViewAssetLoader
import androidx.webkit.WebViewClientCompat
import com.ruzalang.patcher.engine.CustomPackMeta
import com.ruzalang.patcher.engine.LogLevel
import com.ruzalang.patcher.engine.PatchEngine
import com.ruzalang.patcher.engine.PatchLang
import com.ruzalang.patcher.storage.PatcherFileService
import com.ruzalang.patcher.storage.ScanSnapshot
import com.ruzalang.patcher.storage.StorageBridge
import org.json.JSONArray
import org.json.JSONObject
import rikka.shizuku.Shizuku
import java.util.concurrent.Executors

class MainActivity : AppCompatActivity() {

    private lateinit var webView: WebView
    private lateinit var bridge: StorageBridge
    private val worker = Executors.newSingleThreadExecutor()
    private val prefs by lazy { getSharedPreferences("ruza_prefs", Context.MODE_PRIVATE) }

    private val shizukuReqCode = 7001
    private var shizukuBound = false
    @Volatile private var hasScannedOnce = false
    @Volatile private var lastDetectedRunning = false
    @Volatile private var scanBusy = false

    private val userServiceArgs by lazy {
        Shizuku.UserServiceArgs(ComponentName(packageName, PatcherFileService::class.java.name))
            .daemon(false)
            .processNameSuffix("file_service")
            .debuggable(BuildConfig.DEBUG)
            .version(BuildConfig.VERSION_CODE)
    }

    private val shizukuConn = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, service: IBinder?) {
            if (service != null && service.pingBinder()) {
                bridge.shizukuService = IPatcherFileService.Stub.asInterface(service)
                shizukuBound = true
                pushLog(LogLevel.OK, "Shizuku privileged file service connected (No Root).")
                triggerScanAsync()
            }
        }

        override fun onServiceDisconnected(name: ComponentName?) {
            bridge.shizukuService = null
            shizukuBound = false
            pushLog(LogLevel.WARN, "Shizuku service disconnected.")
        }
    }

    private val shizukuPermListener = Shizuku.OnRequestPermissionResultListener { reqCode, grantResult ->
        if (reqCode == shizukuReqCode) {
            if (grantResult == PackageManager.PERMISSION_GRANTED) {
                pushLog(LogLevel.OK, "Shizuku permission granted! Connecting file service...")
                bindShizukuIfReady()
            } else {
                pushLog(LogLevel.WARN, "Shizuku permission was denied.")
                triggerScanAsync()
            }
        }
    }

    private val shizukuBinderReceived = Shizuku.OnBinderReceivedListener {
        bindShizukuIfReady()
    }

    private val shizukuBinderDead = Shizuku.OnBinderDeadListener {
        bridge.shizukuService = null
        shizukuBound = false
    }

    private val customPackPickerLauncher =
        registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
            if (uri == null) return@registerForActivityResult
            worker.execute {
                pushBusy(true)
                try {
                    val fileName = resolveDisplayName(uri) ?: "custom-pack.json"
                    val raw = contentResolver.openInputStream(uri)?.use {
                        it.readBytes().toString(Charsets.UTF_8)
                    } ?: error("Failed reading selected file.")
                    val pack = bridge.saveCustomPack(raw, fileName)
                    pushLog(
                        LogLevel.OK,
                        "Loaded Custom Language Pack: ${pack.meta.name} (${pack.meta.code}) · base=${pack.meta.base.uppercase()} · ${pack.meta.totalCount} overrides ($fileName)",
                    )
                    pushEvent(JSONObject().put("type", "custom_pack").put("pack", customPackMetaToJson(pack.meta)))
                } catch (e: Throwable) {
                    pushLog(LogLevel.ERR, "Custom pack load failed: ${e.message}")
                    pushEvent(
                        JSONObject()
                            .put("type", "alert")
                            .put("title", "Invalid Custom Language Pack")
                            .put("body", e.message ?: "Failed to parse JSON."),
                    )
                } finally {
                    pushBusy(false)
                }
            }
        }

    private fun resolveDisplayName(uri: Uri): String? {
        return try {
            contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
                if (c.moveToFirst()) c.getString(0) else null
            }
        } catch (_: Throwable) {
            null
        }
    }

    private fun customPackMetaToJson(m: CustomPackMeta): JSONObject = JSONObject().apply {
        put("file_name", m.fileName)
        put("name", m.name)
        put("code", m.code)
        put("author", m.author)
        put("base", m.base)
        put("vstring_count", m.vstringCount)
        put("static_ui_count", m.staticUiCount)
        put("builtin_count", m.builtinCount)
        put("total_count", m.totalCount)
    }

    private val safPickerLauncher =
        registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { res ->
            val uri = res.data?.data
            if (res.resultCode == RESULT_OK && uri != null) {
                try {
                    val flags = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                    contentResolver.takePersistableUriPermission(uri, flags)
                    bridge.safTreeUri = uri
                    prefs.edit().putString("saf_uri", uri.toString()).apply()
                    pushLog(LogLevel.OK, "Granted folder access via SAF.")
                } catch (e: Throwable) {
                    pushLog(LogLevel.ERR, "Failed persisting SAF permission: ${e.message}")
                }
            }
            triggerScanAsync()
        }

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        WindowCompat.setDecorFitsSystemWindows(window, false)
        val insetsCtrl = WindowInsetsControllerCompat(window, window.decorView)
        insetsCtrl.isAppearanceLightStatusBars = false
        insetsCtrl.isAppearanceLightNavigationBars = false

        bridge = StorageBridge(this)
        prefs.getString("saf_uri", null)?.let { saved ->
            try {
                bridge.safTreeUri = Uri.parse(saved)
            } catch (_: Throwable) {}
        }

        Shizuku.addRequestPermissionResultListener(shizukuPermListener)
        Shizuku.addBinderReceivedListenerSticky(shizukuBinderReceived)
        Shizuku.addBinderDeadListener(shizukuBinderDead)

        val assetLoader = WebViewAssetLoader.Builder()
            .addPathHandler("/assets/", WebViewAssetLoader.AssetsPathHandler(this))
            .build()

        val root = FrameLayout(this).apply {
            setBackgroundColor(Color.parseColor("#121924"))
        }
        ViewCompat.setOnApplyWindowInsetsListener(root) { v, windowInsets ->
            val bars = windowInsets.getInsets(
                WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout(),
            )
            v.setPadding(bars.left, bars.top, bars.right, bars.bottom)
            windowInsets
        }

        webView = WebView(this).apply {
            setBackgroundColor(Color.parseColor("#141b26"))
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.allowFileAccess = false
            settings.allowContentAccess = false
            settings.cacheMode = WebSettings.LOAD_NO_CACHE
            addJavascriptInterface(JsBridge(), "AndroidBridge")
            webViewClient = object : WebViewClientCompat() {
                override fun shouldInterceptRequest(
                    view: WebView,
                    request: WebResourceRequest,
                ): WebResourceResponse? = assetLoader.shouldInterceptRequest(request.url)
            }
        }
        root.addView(
            webView,
            FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT,
            ),
        )
        setContentView(root)
        ViewCompat.requestApplyInsets(root)
        webView.loadUrl("https://appassets.androidplatform.net/assets/ui/index.html")
    }

    override fun onResume() {
        super.onResume()
        // Auto re-check after returning from App Info (Force Stop) so Install Patch unlocks immediately
        if (hasScannedOnce && lastDetectedRunning && !scanBusy) {
            triggerScanAsync()
        }
    }

    override fun onDestroy() {
        Shizuku.removeRequestPermissionResultListener(shizukuPermListener)
        Shizuku.removeBinderReceivedListener(shizukuBinderReceived)
        Shizuku.removeBinderDeadListener(shizukuBinderDead)
        if (shizukuBound) {
            try {
                Shizuku.unbindUserService(userServiceArgs, shizukuConn, true)
            } catch (_: Throwable) {}
        }
        worker.shutdownNow()
        super.onDestroy()
    }

    private fun isShizukuInstalled(): Boolean {
        return try {
            if (Build.VERSION.SDK_INT >= 33) {
                packageManager.getPackageInfo("moe.shizuku.privileged.api", PackageManager.PackageInfoFlags.of(0))
            } else {
                @Suppress("DEPRECATION")
                packageManager.getPackageInfo("moe.shizuku.privileged.api", 0)
            }
            true
        } catch (_: Throwable) {
            false
        }
    }

    private fun isShizukuRunning(): Boolean = try {
        Shizuku.pingBinder()
    } catch (_: Throwable) {
        false
    }

    private fun isShizukuGranted(): Boolean = try {
        isShizukuRunning() && Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED
    } catch (_: Throwable) {
        false
    }

    private fun bindShizukuIfReady() {
        if (!isShizukuRunning() || !isShizukuGranted()) return
        if (bridge.shizukuService != null) return
        try {
            Shizuku.bindUserService(userServiceArgs, shizukuConn)
        } catch (e: Throwable) {
            pushLog(LogLevel.WARN, "Shizuku bind failed: ${e.message}")
        }
    }

    private fun pushEvent(obj: JSONObject) {
        val js = "window.__onPatcherEvent && window.__onPatcherEvent($obj);"
        runOnUiThread { webView.evaluateJavascript(js, null) }
    }

    private fun pushLog(level: LogLevel, msg: String) {
        pushEvent(JSONObject().apply {
            put("type", "log")
            put("level", level.wire)
            put("message", msg)
        })
    }

    private fun pushBusy(v: Boolean) {
        pushEvent(JSONObject().put("type", "busy").put("value", v))
    }

    private fun pushProgress(cur: Int, tot: Int, stage: String) {
        pushEvent(JSONObject().apply {
            put("type", "progress")
            put("current", cur)
            put("total", tot)
            put("stage", stage)
        })
    }

    private fun snapshotToJson(s: ScanSnapshot): JSONObject {
        val targetsArr = JSONArray()
        for (t in s.targets) {
            targetsArr.put(JSONObject().apply {
                put("id", t.id)
                put("name", t.name)
                put("subtitle", t.subtitle)
                put("string_count", t.stringCount)
                put("state", t.state)
                put("has_backup", t.hasBackup)
                put("sha256", t.sha256)
            })
        }
        val pidsArr = JSONArray()
        s.runningPids.forEach { pidsArr.put(it) }
        val reasonsArr = JSONArray()
        s.versionReasons.forEach { reasonsArr.put(it) }
        return JSONObject().apply {
            put("type", "scan_result")
            put("access_mode", s.accessMode.id)
            put("access_label", s.accessMode.label)
            put("shizuku_installed", s.shizukuInstalled)
            put("shizuku_running", s.shizukuRunning)
            put("shizuku_granted", s.shizukuGranted)
            put("game_installed", s.gameInstalled)
            put("running_pids", pidsArr)
            put("version", JSONObject().apply {
                put("detected_version", s.detectedVersion)
                put("expected_version", s.expectedVersion)
                put("detected_res_rev", s.detectedResRev)
                put("expected_res_rev", s.expectedResRev)
                put("status", s.versionStatus)
                put("reasons", reasonsArr)
            })
            put("targets", targetsArr)
            put("news_cache_bytes", s.newsCacheBytes)
            put("has_news_backup", s.hasNewsBackup)
            put("backup_dir", s.backupDirPath)
        }
    }

    private fun triggerScanAsync() {
        worker.execute {
            scanBusy = true
            pushBusy(true)
            try {
                if (isShizukuGranted() && bridge.shizukuService == null) {
                    runOnUiThread { bindShizukuIfReady() }
                    Thread.sleep(350)
                }
                val snap = bridge.scan(
                    shizukuInstalled = isShizukuInstalled(),
                    shizukuRunning = isShizukuRunning(),
                    shizukuGranted = isShizukuGranted(),
                    log = { m, l -> pushLog(l, m) },
                )
                lastDetectedRunning = snap.runningPids.isNotEmpty()
                hasScannedOnce = true
                pushEvent(snapshotToJson(snap))
            } catch (e: Throwable) {
                pushLog(LogLevel.ERR, "Scan failed: ${e.message}")
            } finally {
                scanBusy = false
                pushBusy(false)
            }
        }
    }

    private fun openSafeUrl(url: String) {
        val allowed = url.startsWith("https://github.com/notnedar/ruza-lang") ||
            url.startsWith("https://shizuku.rikka.app") ||
            url.startsWith("https://play.google.com/store/apps/details?id=moe.shizuku.privileged.api")
        if (!allowed) return
        try {
            startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
        } catch (_: Throwable) {}
    }

    private fun launchSafPicker() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
            addFlags(
                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                    Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
                    Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION,
            )
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                val initUri = DocumentsContract.buildDocumentUri(
                    "com.android.externalstorage.documents",
                    "primary:Android/data/${PatchEngine.ANDROID_PACKAGE_ID}",
                )
                putExtra(DocumentsContract.EXTRA_INITIAL_URI, initUri)
            }
        }
        try {
            safPickerLauncher.launch(intent)
        } catch (e: Throwable) {
            pushLog(LogLevel.ERR, "Could not open folder picker: ${e.message}")
        }
    }

    inner class JsBridge {
        @JavascriptInterface
        fun postMessage(rawJson: String) {
            val req = try {
                JSONObject(rawJson)
            } catch (_: Throwable) {
                return
            }
            val cmd = req.optString("cmd", "")
            val lang = PatchLang.fromCode(req.optString("lang", "en"))

            when (cmd) {
                "init" -> {
                    pushLog(LogLevel.INFO, "Ruza-Lang Android v${BuildConfig.VERSION_NAME} (EN / ID + Custom Packs) initialized.")
                    pushLog(LogLevel.INFO, "Safety: Exact byte sizes · Zero root · Zero APK re-signing.")
                    bridge.loadPersistedCustomPack()?.let { saved ->
                        pushLog(
                            LogLevel.OK,
                            "Restored Custom Language Pack: ${saved.meta.name} (${saved.meta.code}) · ${saved.meta.totalCount} overrides.",
                        )
                        pushEvent(JSONObject().put("type", "custom_pack").put("pack", customPackMetaToJson(saved.meta)))
                    }
                    runOnUiThread { bindShizukuIfReady() }
                    triggerScanAsync()
                    // Silent background update check
                    Thread {
                        try {
                            val info = UpdateChecker.check(BuildConfig.VERSION_NAME)
                            if (info.hasUpdate) {
                                pushLog(LogLevel.INFO, "New release available: ${info.latestVersion} (current: ${info.currentVersion}).")
                            }
                            pushEvent(JSONObject().apply {
                                put("type", "update_status")
                                put("silent", true)
                                put("info", JSONObject().apply {
                                    put("current_version", info.currentVersion)
                                    put("latest_version", info.latestVersion)
                                    put("has_update", info.hasUpdate)
                                    put("release_url", info.releaseUrl)
                                    put("apk_url", info.apkUrl)
                                    put("apk_size", info.apkSize)
                                })
                            })
                        } catch (_: Throwable) {}
                    }.start()
                }

                "scan" -> triggerScanAsync()

                "request_shizuku" -> {
                    runOnUiThread {
                        if (!isShizukuRunning()) {
                            val title = if (lang == PatchLang.ID) "Shizuku Belum Berjalan" else "Shizuku Not Running"
                            val body = if (lang == PatchLang.ID) {
                                "Buka aplikasi Shizuku dulu lalu klik Start lewat Wireless Debugging di HP (tanpa PC & tanpa root), lalu kembali ke Ruza-Lang dan ketuk Izinkan Shizuku."
                            } else {
                                "Open the Shizuku app and start it via Wireless Debugging on your phone (no PC & no root), then return to Ruza-Lang and tap Authorize Shizuku."
                            }
                            pushEvent(JSONObject().put("type", "alert").put("title", title).put("body", body))
                            return@runOnUiThread
                        }
                        if (isShizukuGranted()) {
                            bindShizukuIfReady()
                            triggerScanAsync()
                        } else {
                            try {
                                Shizuku.requestPermission(shizukuReqCode)
                            } catch (e: Throwable) {
                                pushLog(LogLevel.ERR, "Shizuku request failed: ${e.message}")
                            }
                        }
                    }
                }

                "open_shizuku" -> {
                    runOnUiThread {
                        val launch = packageManager.getLaunchIntentForPackage("moe.shizuku.privileged.api")
                        if (launch != null) {
                            startActivity(launch)
                        } else {
                            openSafeUrl("https://shizuku.rikka.app/download/")
                        }
                    }
                }

                "pick_saf" -> runOnUiThread { launchSafPicker() }

                "pick_custom_pack" -> {
                    runOnUiThread {
                        try {
                            customPackPickerLauncher.launch(arrayOf("application/json", "text/plain", "*/*"))
                        } catch (e: Throwable) {
                            pushLog(LogLevel.ERR, "Could not open file picker: ${e.message}")
                        }
                    }
                }

                "clear_custom_pack" -> {
                    worker.execute {
                        bridge.clearCustomPack()
                        pushLog(LogLevel.INFO, "Cleared Custom Language Pack — reverted to built-in EN / ID.")
                        pushEvent(JSONObject().put("type", "custom_pack").put("pack", JSONObject.NULL))
                    }
                }

                "open_app_settings" -> {
                    runOnUiThread {
                        try {
                            val intent = Intent(
                                Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                                Uri.parse("package:${PatchEngine.ANDROID_PACKAGE_ID}"),
                            )
                            startActivity(intent)
                        } catch (_: Throwable) {}
                    }
                }

                "patch" -> {
                    worker.execute {
                        pushBusy(true)
                        try {
                            val custom = bridge.loadPersistedCustomPack()
                            val n = bridge.applyPatch(
                                lang = lang,
                                custom = custom,
                                log = { m, l -> pushLog(l, m) },
                                onProgress = { c, t, s -> pushProgress(c, t, s) },
                            )
                            val (title, body) = if (custom != null) {
                                if (lang == PatchLang.ID) {
                                    "Patch Bahasa Kustom (${custom.meta.code}) Selesai!" to
                                        "Paket kustom ${custom.meta.name} (${custom.meta.code}) berhasil dipasang & diverifikasi 100%!\nBasis: ${custom.meta.base.uppercase()} · Override: ${custom.meta.totalCount} · Total di-rewrite: $n strings."
                                } else {
                                    "Custom Pack (${custom.meta.code}) Complete!" to
                                        "Custom pack ${custom.meta.name} (${custom.meta.code}) installed & verified 100%!\nBase: ${custom.meta.base.uppercase()} · Overrides: ${custom.meta.totalCount} · Total rewritten: $n strings."
                                }
                            } else if (lang == PatchLang.ID) {
                                "Patch Bahasa Indonesia Selesai!" to
                                    "Patch Bahasa Indonesia (ID) berhasil dipasang & diverifikasi 100%!\nTotal string di-rewrite: $n strings."
                            } else {
                                "English Patch Complete!" to
                                    "English (EN) patch installed & verified 100%!\nTotal strings rewritten: $n strings."
                            }
                            pushEvent(JSONObject().put("type", "alert").put("title", title).put("body", body))
                        } catch (e: Throwable) {
                            pushLog(LogLevel.ERR, "PATCH FAILED: ${e.message}")
                            pushEvent(JSONObject().put("type", "alert").put("title", "Patch Failed").put("body", e.message ?: "Error"))
                        } finally {
                            triggerScanAsync()
                        }
                    }
                }

                "patch_news" -> {
                    worker.execute {
                        pushBusy(true)
                        try {
                            val (patched, total) = bridge.applyNewsPatch(
                                lang = lang,
                                log = { m, l -> pushLog(l, m) },
                                onProgress = { c, t, s -> pushProgress(c, t, s) },
                            )
                            val (title, body) = if (lang == PatchLang.ID) {
                                "News Bahasa Indonesia Selesai!" to
                                    "Berhasil menerjemahkan $patched/$total pengumuman cache in-game ke Bahasa Indonesia (ID)."
                            } else {
                                "News Translation Complete!" to
                                    "Successfully translated $patched/$total cached in-game announcements to English (EN)."
                            }
                            pushEvent(JSONObject().put("type", "alert").put("title", title).put("body", body))
                        } catch (e: Throwable) {
                            pushLog(LogLevel.ERR, "NEWS PATCH FAILED: ${e.message}")
                            pushEvent(JSONObject().put("type", "alert").put("title", "Translate News Failed").put("body", e.message ?: "Error"))
                        } finally {
                            triggerScanAsync()
                        }
                    }
                }

                "rollback" -> {
                    worker.execute {
                        pushBusy(true)
                        try {
                            val n = bridge.rollbackToOriginal(
                                log = { m, l -> pushLog(l, m) },
                                onProgress = { c, t, s -> pushProgress(c, t, s) },
                            )
                            val (title, body) = if (lang == PatchLang.ID) {
                                "Rollback Selesai" to
                                    "Berhasil mengembalikan $n/2 bundle ke Original JP dengan SHA-256 terverifikasi."
                            } else {
                                "Rollback Complete" to
                                    "Successfully restored $n/2 bundles to Original JP with verified SHA-256 hashes."
                            }
                            pushEvent(JSONObject().put("type", "alert").put("title", title).put("body", body))
                        } catch (e: Throwable) {
                            pushLog(LogLevel.ERR, "ROLLBACK FAILED: ${e.message}")
                            pushEvent(JSONObject().put("type", "alert").put("title", "Rollback Failed").put("body", e.message ?: "Error"))
                        } finally {
                            triggerScanAsync()
                        }
                    }
                }

                "check_update" -> {
                    Thread {
                        pushEvent(JSONObject().put("type", "update_checking").put("value", true))
                        try {
                            val info = UpdateChecker.check(BuildConfig.VERSION_NAME)
                            pushEvent(JSONObject().apply {
                                put("type", "update_status")
                                put("silent", false)
                                put("info", JSONObject().apply {
                                    put("current_version", info.currentVersion)
                                    put("latest_version", info.latestVersion)
                                    put("has_update", info.hasUpdate)
                                    put("release_url", info.releaseUrl)
                                    put("apk_url", info.apkUrl)
                                    put("apk_size", info.apkSize)
                                })
                            })
                        } catch (e: Throwable) {
                            pushEvent(JSONObject().put("type", "update_error").put("message", e.message ?: "Check failed"))
                        } finally {
                            pushEvent(JSONObject().put("type", "update_checking").put("value", false))
                        }
                    }.start()
                }

                "copy_text" -> {
                    val txt = req.optString("text", "")
                    runOnUiThread {
                        val cm = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                        cm.setPrimaryClip(ClipData.newPlainText("Ruza-Lang Log", txt))
                    }
                }

                "open_url" -> {
                    val url = req.optString("url", "")
                    runOnUiThread { openSafeUrl(url) }
                }
            }
        }
    }
}
