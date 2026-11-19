package dev.metrolist.desktop

import io.ktor.client.HttpClient
import io.ktor.client.engine.okhttp.OkHttp
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.plugins.contentnegotiation.ContentNegotiation
import io.ktor.client.request.get
import io.ktor.client.request.header
import io.ktor.client.request.post
import io.ktor.client.request.setBody
import io.ktor.client.statement.bodyAsText
import io.ktor.http.ContentType
import io.ktor.http.contentType
import io.ktor.http.isSuccess
import io.ktor.serialization.kotlinx.json.json
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.add
import kotlinx.serialization.json.addJsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject
import java.security.MessageDigest
import java.util.Locale

/** Batch playlist writes that the older upstream facade only exposes one video at a time. */
internal object YouTubeWrite {
    private const val BASE = "https://music.youtube.com/youtubei/v1"
    private const val ORIGIN = "https://music.youtube.com"
    private const val FALLBACK_VERSION = "1.20250101.01.00"
    private const val USER_AGENT =
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 " +
            "(KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36"
    private const val CLIENT_NAME = "67"

    private val json = Json { ignoreUnknownKeys = true }
    private val client = HttpClient(OkHttp) {
        install(ContentNegotiation) { json(json) }
        install(HttpTimeout) {
            connectTimeoutMillis = 15_000
            requestTimeoutMillis = 30_000
            socketTimeoutMillis = 30_000
        }
    }

    @Volatile private var cachedVersion: String? = null

    /** Creates a private playlist, optionally seeded with all `videoIds` in the same round trip. */
    suspend fun createPlaylist(title: String, videoIds: List<String>, cookie: String, visitorData: String?, dataSyncId: String?): String {
        val version = clientVersion(cookie)
        val response = client.post("$BASE/playlist/create?prettyPrint=false") {
            contentType(ContentType.Application.Json)
            authHeaders(cookie, visitorData)
            header("X-YouTube-Client-Name", CLIENT_NAME)
            header("X-YouTube-Client-Version", version)
            setBody(
                buildJsonObject {
                    context(version, visitorData, dataSyncId)
                    put("title", title)
                    put("description", "Imported from Spotify")
                    put("privacyStatus", "PRIVATE")
                    if (videoIds.isNotEmpty()) putJsonArray("videoIds") { videoIds.distinct().forEach { add(it) } }
                },
            )
        }
        val text = response.bodyAsText()
        if (!response.status.isSuccess()) error("YouTube Music rejected the playlist create (HTTP ${response.status.value})")
        val root = runCatching { json.parseToJsonElement(text) as? JsonObject }.getOrNull()
            ?: error("YouTube Music returned an unreadable playlist create")
        return findPlaylistId(root) ?: error("YouTube Music created a playlist but returned no id")
    }

    /** Returns the number of entries accepted; throws if YouTube refuses the whole edit. */
    suspend fun addAll(playlistId: String, videoIds: List<String>, cookie: String, visitorData: String?, dataSyncId: String?): Int {
        if (videoIds.isEmpty()) return 0
        val version = clientVersion(cookie)
        val response = client.post("$BASE/browse/edit_playlist?prettyPrint=false") {
            contentType(ContentType.Application.Json)
            authHeaders(cookie, visitorData)
            header("X-YouTube-Client-Name", CLIENT_NAME)
            header("X-YouTube-Client-Version", version)
            setBody(
                buildJsonObject {
                    context(version, visitorData, dataSyncId)
                    put("playlistId", playlistId.removePrefix("VL"))
                    putJsonArray("actions") {
                        videoIds.distinct().forEach { videoId ->
                            addJsonObject {
                                put("action", "ACTION_ADD_VIDEO")
                                put("addedVideoId", videoId)
                            }
                        }
                    }
                },
            )
        }
        val text = response.bodyAsText()
        if (!response.status.isSuccess()) error("YouTube Music rejected the playlist edit (HTTP ${response.status.value})")
        val root = runCatching { json.parseToJsonElement(text) as? JsonObject }.getOrNull()
            ?: error("YouTube Music returned an unreadable playlist edit")
        val status = root["status"]?.toString()?.trim('"')
        if (status != null && status != "STATUS_SUCCEEDED") error("YouTube Music refused the edit ($status)")
        val results = root["playlistEditResults"] as? JsonArray
        return results?.size ?: videoIds.distinct().size
    }

    private fun kotlinx.serialization.json.JsonObjectBuilder.context(version: String, visitorData: String?, dataSyncId: String?) {
        putJsonObject("context") {
            putJsonObject("client") {
                put("clientName", "WEB_REMIX")
                put("clientVersion", version)
                put("hl", "en")
                put("gl", "US")
                visitorData?.let { put("visitorData", it) }
            }
            putJsonObject("user") {
                put("lockedSafetyMode", false)
                dataSyncId?.let { put("onBehalfOfUser", it) }
            }
            putJsonObject("request") { put("useSsl", true) }
        }
    }

    private fun findPlaylistId(value: kotlinx.serialization.json.JsonElement): String? = when (value) {
        is JsonObject -> value["playlistId"]?.let { it as? kotlinx.serialization.json.JsonPrimitive }?.content?.takeIf { it.isNotBlank() }
            ?: value.values.firstNotNullOfOrNull { findPlaylistId(it) }
        is JsonArray -> value.firstNotNullOfOrNull { findPlaylistId(it) }
        else -> null
    }

    private suspend fun clientVersion(cookie: String): String {
        cachedVersion?.let { return it }
        val version = runCatching {
            val html = client.get("$ORIGIN/") {
                header("Cookie", cookie)
                header("Accept-Language", "en-US,en;q=0.9")
                header("User-Agent", USER_AGENT)
            }.bodyAsText()
            Regex(""""INNERTUBE_CLIENT_VERSION"\s*:\s*"([^"]+)"""").find(html)?.groupValues?.get(1)?.takeIf { it.length > 3 }
        }.getOrNull() ?: FALLBACK_VERSION
        cachedVersion = version
        return version
    }

    private fun io.ktor.client.request.HttpRequestBuilder.authHeaders(cookie: String, visitorData: String?) {
        header("X-Origin", ORIGIN)
        header("Origin", ORIGIN)
        header("Referer", "$ORIGIN/")
        header("Accept-Language", "en-US,en;q=0.9")
        header("User-Agent", USER_AGENT)
        header("Cookie", cookie)
        header("X-Goog-AuthUser", "0")
        visitorData?.let { header("X-Goog-Visitor-Id", it) }
        sapisid(cookie)?.let { header("Authorization", hash(it)) }
    }

    private fun sapisid(cookie: String): String? {
        val jar = cookie.split(';').mapNotNull { entry ->
            val name = entry.substringBefore('=').trim()
            val value = entry.substringAfter('=', "").trim()
            if (name.isEmpty() || value.isEmpty()) null else name to value
        }.toMap()
        return listOf("SAPISID", "__Secure-3PAPISID", "__Secure-1PAPISID").firstNotNullOfOrNull { jar[it] }
    }

    private fun hash(sapisid: String): String {
        val timestamp = System.currentTimeMillis() / 1000
        val digest = MessageDigest.getInstance("SHA-1")
            .digest("$timestamp $sapisid $ORIGIN".toByteArray())
            .joinToString("") { "%02x".format(Locale.ROOT, it) }
        return "SAPISIDHASH ${timestamp}_$digest"
    }
}
