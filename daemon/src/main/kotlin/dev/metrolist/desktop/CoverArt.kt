package dev.metrolist.desktop

import io.ktor.client.HttpClient
import io.ktor.client.engine.okhttp.OkHttp
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.plugins.defaultRequest
import io.ktor.client.request.get
import io.ktor.client.request.head
import io.ktor.client.request.header
import io.ktor.client.statement.bodyAsText
import io.ktor.http.isSuccess
import io.ktor.http.encodeURLParameter
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.util.concurrent.ConcurrentHashMap

@Serializable data class CoverDto(val url: String?)

/** Public cover art (MusicBrainz + Cover Art Archive, then the iTunes Search catalogue) so Discord never sees a server's credentialed or LAN cover URL. */
object CoverArt {
    internal var mbBase = "https://musicbrainz.org"
    internal var caaBase = "https://coverartarchive.org"
    internal var itunesBase = "https://itunes.apple.com"
    private const val MISS_MS = 3_600_000L

    private val http = HttpClient(OkHttp) {
        install(HttpTimeout) { connectTimeoutMillis = 8_000; requestTimeoutMillis = 8_000; socketTimeoutMillis = 8_000 }
        defaultRequest { header("User-Agent", "Serafin/1.0 (https://github.com/hireri/serafin)") }
    }
    private val gate = RateGate(1, 1_100)
    // ponytail: unbounded map; a desktop session sees few distinct songs, add eviction if that changes.
    private val cache = ConcurrentHashMap<String, Pair<String?, Long>>()

    internal fun clear() = cache.clear()

    suspend fun lookup(artist: String, album: String, title: String): CoverDto {
        val key = "${artist.lowercase()}|${album.lowercase()}|${title.lowercase()}"
        cache[key]?.let { (url, at) -> if (url != null || System.currentTimeMillis() - at < MISS_MS) return CoverDto(url) }
        val url = runCatching { find(artist.trim(), album.trim(), title.trim()) }.getOrNull()
        cache[key] = url to System.currentTimeMillis()
        return CoverDto(url)
    }

    private fun q(s: String) = s.replace("\\", " ").replace("\"", " ")

    private suspend fun find(artist: String, album: String, title: String): String? {
        if (artist.isBlank() || (album.isBlank() && title.isBlank())) return null
        return runCatching { musicBrainz(artist, album, title) }.getOrNull() ?: runCatching { itunes(artist, album, title) }.getOrNull()
    }

    /** The first of the best three releases that has front art; many releases have none. */
    private suspend fun musicBrainz(artist: String, album: String, title: String): String? {
        val ids = if (album.isNotBlank()) search("release", "release:\"${q(album)}\" AND artist:\"${q(artist)}\"").map { it["id"] }
        else search("recording", "recording:\"${q(title)}\" AND artist:\"${q(artist)}\"").mapNotNull { it["releases"]?.jsonArray?.firstOrNull()?.jsonObject?.get("id") }
        for (mbid in ids) {
            val id = (mbid as? JsonPrimitive)?.contentOrNull ?: continue
            val url = "$caaBase/release/$id/front-500"
            if (runCatching { http.head(url).status.isSuccess() }.getOrDefault(false)) return url
        }
        return null
    }

    /** Apple's public catalogue search: no key, and most released music is in it. The hit's artist must match. */
    private suspend fun itunes(artist: String, album: String, title: String): String? {
        val term = "$artist ${album.ifBlank { title }}"
        val res = http.get("$itunesBase/search?term=${term.encodeURLParameter()}&entity=${if (album.isNotBlank()) "album" else "song"}&limit=5")
        if (!res.status.isSuccess()) return null
        val a = artist.lowercase()
        return Json.parseToJsonElement(res.bodyAsText()).jsonObject["results"]?.jsonArray.orEmpty().map { it.jsonObject }
            .firstOrNull { h -> h["artistName"]?.jsonPrimitive?.contentOrNull?.lowercase()?.let { it.contains(a) || a.contains(it) } == true }
            ?.get("artworkUrl100")?.jsonPrimitive?.contentOrNull?.replace(Regex("""/\d+x\d+bb"""), "/600x600bb")
    }

    /** MusicBrainz's best three hits, best first. */
    private suspend fun search(kind: String, query: String): List<JsonObject> {
        val body = gate.run { runCatching { http.get("$mbBase/ws/2/$kind/?query=${query.encodeURLParameter()}&fmt=json&limit=3").also { check(it.status.isSuccess()) }.bodyAsText() } }
            .getOrThrow()
        return Json.parseToJsonElement(body).jsonObject["${kind}s"]?.jsonArray.orEmpty().map { it.jsonObject }
    }
}
