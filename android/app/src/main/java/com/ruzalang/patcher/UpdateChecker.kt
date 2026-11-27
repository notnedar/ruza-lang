package com.ruzalang.patcher

import org.json.JSONObject
import java.io.BufferedReader
import java.io.InputStreamReader
import java.net.HttpURLConnection
import java.net.URL

data class ApkUpdateInfo(
    val currentVersion: String,
    val latestVersion: String,
    val hasUpdate: Boolean,
    val releaseUrl: String,
    val apkUrl: String,
    val apkSize: Long,
)

object UpdateChecker {
    private const val API_URL = "https://api.github.com/repos/notnedar/ruza-lang/releases/latest"

    private fun parseSemver(v: String): Triple<Int, Int, Int>? {
        val core = v.trim().trimStart('v', 'V').substringBefore('-').substringBefore('+')
        val p = core.split('.')
        val maj = p.getOrNull(0)?.toIntOrNull() ?: return null
        val min = p.getOrNull(1)?.toIntOrNull() ?: 0
        val pat = p.getOrNull(2)?.toIntOrNull() ?: 0
        return Triple(maj, min, pat)
    }

    fun isNewer(latest: String, current: String): Boolean {
        val l = parseSemver(latest) ?: return false
        val c = parseSemver(current) ?: return false
        if (l.first != c.first) return l.first > c.first
        if (l.second != c.second) return l.second > c.second
        return l.third > c.third
    }

    fun check(currentVersion: String): ApkUpdateInfo {
        val conn = (URL(API_URL).openConnection() as HttpURLConnection).apply {
            requestMethod = "GET"
            connectTimeout = 10000
            readTimeout = 15000
            setRequestProperty("Accept", "application/vnd.github+json")
            setRequestProperty("User-Agent", "Ruza-Lang-Android/$currentVersion")
        }
        try {
            val code = conn.responseCode
            if (code != 200) error("GitHub API returned HTTP $code")
            val body = BufferedReader(InputStreamReader(conn.inputStream)).use { it.readText() }
            val json = JSONObject(body)
            val tag = json.optString("tag_name", "").trim()
            val htmlUrl = json.optString("html_url", "https://github.com/notnedar/ruza-lang/releases")
            var apkUrl = ""
            var apkSize = 0L
            val assets = json.optJSONArray("assets")
            if (assets != null) {
                for (i in 0 until assets.length()) {
                    val a = assets.getJSONObject(i)
                    val name = a.optString("name", "")
                    if (name.endsWith(".apk", ignoreCase = true)) {
                        apkUrl = a.optString("browser_download_url", "")
                        apkSize = a.optLong("size", 0L)
                        break
                    }
                }
            }
            val curTag = if (currentVersion.startsWith("v")) currentVersion else "v$currentVersion"
            val newer = isNewer(tag, currentVersion)
            return ApkUpdateInfo(
                currentVersion = curTag,
                latestVersion = tag.ifEmpty { curTag },
                hasUpdate = newer,
                releaseUrl = htmlUrl,
                apkUrl = apkUrl,
                apkSize = apkSize,
            )
        } finally {
            conn.disconnect()
        }
    }
}
