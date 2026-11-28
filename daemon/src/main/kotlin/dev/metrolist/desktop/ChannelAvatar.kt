package dev.metrolist.desktop

import com.metrolist.innertube.YouTube
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit

/**
 * The square avatar of an artist's YouTube channel. YouTube Music itself only has one picture per artist, in whatever
 * shape was uploaded, which the artist page uses as its banner.
 */
object ChannelAvatar {
    // Short, since the artist page waits for it and can do without.
    private val http = OkHttpClient.Builder().callTimeout(5, TimeUnit.SECONDS).build()
    private val cache = ConcurrentHashMap<String, String>()

    /** Anonymous; null when the channel has none or can't be reached. */
    suspend fun of(channelId: String): String? = cache[channelId] ?: withContext(Dispatchers.IO) {
        runCatching {
            val client = http.newBuilder().proxy(YouTube.proxy).apply {
                YouTube.proxyAuth?.let { auth ->
                    proxyAuthenticator { _, response -> response.request.newBuilder().header("Proxy-Authorization", auth).build() }
                }
            }.build()
            val body = """{"context":{"client":{"clientName":"WEB","clientVersion":"2.20250101.00.00"}},"browseId":${Json.encodeToString(channelId)}}"""
            val request = Request.Builder()
                .url("https://www.youtube.com/youtubei/v1/browse?prettyPrint=false")
                .post(body.toRequestBody("application/json".toMediaType()))
                .build()
            client.newCall(request).execute().use { parse(it.body.string()) }
        }.getOrNull()?.also { cache[channelId] = it }
    }

    /** The avatar in a channel's browse response, as a 1200px square in the `=wN-hN` form the client can resize. */
    internal fun parse(response: String): String? {
        val meta = (Json.parseToJsonElement(response) as? JsonObject)?.get("metadata")?.jsonObject?.get("channelMetadataRenderer")
        val url = meta?.jsonObject?.get("avatar")?.jsonObject?.get("thumbnails")?.jsonArray
            ?.lastOrNull()?.jsonObject?.get("url")?.jsonPrimitive?.content ?: return null
        return url.substringBefore('=') + "=w1200-h1200-l90-rj"
    }
}
