package dev.metrolist.desktop

import io.ktor.client.HttpClient
import io.ktor.client.engine.okhttp.OkHttp
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.request.forms.MultiPartFormDataContent
import io.ktor.client.request.forms.formData
import io.ktor.client.request.get
import io.ktor.client.request.header
import io.ktor.client.request.post
import io.ktor.client.request.setBody
import io.ktor.http.ContentType
import io.ktor.http.Headers
import io.ktor.http.HttpHeaders
import io.ktor.http.contentType
import io.ktor.http.isSuccess
import kotlinx.serialization.json.buildJsonObject
import io.ktor.client.statement.bodyAsText
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.longOrNull
import kotlinx.coroutines.CancellationException
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import java.net.URLEncoder
import java.security.MessageDigest
import java.security.SecureRandom

/** Navidrome / any OpenSubsonic server, Subsonic API 1.16.1 with token auth. */
class NavidromeSource(override val config: SourceConfig) : MusicSource {
    override val kind = Kind.LIBRARY
    override val caps = setOf(Cap.SEARCH, Cap.HOME, Cap.LIBRARY, Cap.LIKE, Cap.RADIO, Cap.SCROBBLE, Cap.PLAYLIST_WRITE, Cap.IMAGE)

    private class SubsonicError(val code: Int, message: String) : Exception(message)

    private val base = config.url.orEmpty().trim().trimEnd('/')
    private val rng = SecureRandom()

    private fun enc(s: String) = URLEncoder.encode(s, "UTF-8")

    private fun url(method: String, vararg params: Pair<String, String>): String {
        val salt = ByteArray(8).also(rng::nextBytes).joinToString("") { "%02x".format(it) }
        val token = MessageDigest.getInstance("MD5").digest((config.secret.orEmpty() + salt).toByteArray()).joinToString("") { "%02x".format(it) }
        val q = listOf("u" to config.username.orEmpty(), "t" to token, "s" to salt, "v" to "1.16.1", "c" to "serafin", "f" to "json") + params
        return "$base/rest/$method.view?" + q.joinToString("&") { "${it.first}=${enc(it.second)}" }
    }

    private suspend fun call(method: String, vararg params: Pair<String, String>): JsonObject {
        val body = http.get(url(method, *params)).bodyAsText()
        val resp = runCatching { Json.parseToJsonElement(body).jsonObject["subsonic-response"]?.jsonObject }.getOrNull()
            ?: throw SubsonicError(-1, "Not a Subsonic server")
        if (resp.str("status") == "failed") {
            val e = resp["error"] as? JsonObject
            throw SubsonicError(e?.int("code") ?: 0, e?.str("message") ?: "Request failed")
        }
        return resp
    }

    override suspend fun test(): TestResultDto = try {
        val r = call("ping")
        val name = r.str("type")?.replaceFirstChar { it.uppercase() } ?: "Subsonic"
        TestResultDto(true, detail = listOfNotNull(name, r.str("serverVersion")).joinToString(" "))
    } catch (e: SubsonicError) {
        TestResultDto(false, error = if (e.code == 40 || e.code == 41) "Wrong username or password" else e.message)
    } catch (e: IllegalArgumentException) {
        TestResultDto(false, error = "Invalid server URL")
    } catch (e: Exception) {
        TestResultDto(false, error = "Can't reach the server")
    }

    private fun cover(o: JsonObject) = o.str("coverArt")?.let { url("getCoverArt", "id" to it, "size" to "300") }

    private fun artistsOf(o: JsonObject): List<String> =
        (o["artists"] as? JsonArray)?.mapNotNull { (it as? JsonObject)?.str("name") }?.takeIf { it.isNotEmpty() }
            ?: listOfNotNull(o.str("artist"))

    private fun track(o: JsonObject) = TrackDto(
        id = o.str("id").orEmpty(), title = o.str("title").orEmpty(), artists = artistsOf(o), album = o.str("album"),
        durationSec = o.int("duration"), thumbnail = cover(o), source = config.id, artistId = o.str("artistId"), albumId = o.str("albumId"),
        liked = o.str("starred") != null,
    )

    private fun songItem(o: JsonObject) = ItemDto(
        "song", o.str("id").orEmpty(), o.str("title").orEmpty(), artistsOf(o).joinToString(", ").ifEmpty { null }, cover(o),
        o.int("duration"), o.str("artistId"), o.str("album"), o.str("albumId"),
    )

    private fun albumItem(o: JsonObject) = ItemDto(
        "album", o.str("id").orEmpty(), o.str("name") ?: o.str("title").orEmpty(),
        listOfNotNull(o.str("artist"), o.int("year")?.toString()).joinToString(" · ").ifEmpty { null }, cover(o), artistId = o.str("artistId"),
    )

    private fun artistItem(o: JsonObject) = ItemDto(
        "artist", o.str("id").orEmpty(), o.str("name").orEmpty(), o.int("albumCount")?.let { "$it albums" }, cover(o),
    )

    private fun playlistItem(o: JsonObject) = ItemDto(
        "playlist", o.str("id").orEmpty(), o.str("name").orEmpty(), o.int("songCount")?.let { "$it songs" }, cover(o),
        // Only the owner edits a playlist (an admin could too, but the server will say so).
        editable = o.str("owner")?.equals(config.username, ignoreCase = true) ?: true,
    )

    private fun JsonObject.list(key: String) = (this[key] as? JsonArray)?.mapNotNull { it as? JsonObject }.orEmpty()
    private fun JsonObject.obj(key: String) = this[key] as? JsonObject ?: JsonObject(emptyMap())

    override suspend fun search(q: String, type: String): List<ItemDto> {
        if (type == "playlist") {
            return call("getPlaylists").obj("playlists").list("playlist").filter { it.str("name").orEmpty().contains(q, ignoreCase = true) }.map(::playlistItem)
        }
        if (type != "song" && type != "album" && type != "artist") return emptyList()
        val none = "0"
        val r = call(
            "search3", "query" to q, "songCount" to if (type == "song") "50" else none,
            "albumCount" to if (type == "album") "50" else none, "artistCount" to if (type == "artist") "50" else none,
        ).obj("searchResult3")
        return when (type) {
            "song" -> r.list("song").map(::songItem)
            "album" -> r.list("album").map(::albumItem)
            else -> r.list("artist").map(::artistItem)
        }
    }

    override suspend fun library(kind: String): List<ItemDto> = when (kind) {
        "songs" -> call("getStarred2").obj("starred2").list("song").map(::songItem)
        "albums" -> albumList("alphabeticalByName", 5000)
        "artists" -> call("getArtists").obj("artists").list("index").flatMap { it.list("artist") }.map(::artistItem)
        "playlists" -> listOf(STARRED_ITEM) + call("getPlaylists").obj("playlists").list("playlist").map(::playlistItem)
        else -> emptyList()
    }

    // ponytail: capped at 5000 albums (10 pages); stream pages lazily if a library is bigger
    private suspend fun albumList(type: String, max: Int): List<ItemDto> {
        val out = mutableListOf<ItemDto>()
        while (out.size < max) {
            val page = call("getAlbumList2", "type" to type, "size" to minOf(500, max - out.size).toString(), "offset" to out.size.toString())
                .obj("albumList2").list("album")
            out += page.map(::albumItem)
            if (page.size < 500) break
        }
        return out
    }

    override suspend fun home(): List<SectionDto> = listOf(
        Triple("Recently added", "recent", "newest"), Triple("Most played", "played", "frequent"),
        Triple("Recently played", "continue", "recent"), Triple("Random albums", "mixes", "random"),
    ).mapNotNull { (title, shelf, type) ->
        val items = runCatching { albumList(type, 20) }.getOrDefault(emptyList())
        items.takeIf { it.isNotEmpty() }?.let { SectionDto(title, it, shelf) }
    }

    override suspend fun album(id: String): PageDto {
        val a = call("getAlbum", "id" to id).obj("album")
        return PageDto(albumItem(a), tracks = a.list("song").map(::track))
    }

    override suspend fun artist(id: String): PageDto {
        val a = call("getArtist", "id" to id).obj("artist")
        val albums = a.list("album").sortedWith(compareByDescending<JsonObject> { it.int("year") ?: 0 }.thenBy { it.str("name").orEmpty().lowercase() })
        return PageDto(artistItem(a), sections = listOf(SectionDto("Albums", albums.map(::albumItem))).filter { it.items.isNotEmpty() })
    }

    override suspend fun playlist(id: String): PageDto {
        if (id == STARRED) return PageDto(STARRED_ITEM, tracks = call("getStarred2").obj("starred2").list("song").map(::track))
        val p = call("getPlaylist", "id" to id).obj("playlist")
        val owner = p.str("owner")
        val item = playlistItem(p).copy(author = owner, authorAvatar = owner?.let { url("getAvatar", "username" to it) })
        return PageDto(item, description = p.str("comment"), tracks = p.list("entry").map(::track))
    }

    override suspend fun stream(id: String): StreamDto {
        val s = call("getSong", "id" to id).obj("song")
        val gain = (s["replayGain"] as? JsonObject)?.get("trackGain")?.let { (it as? JsonPrimitive)?.doubleOrNull }
        return StreamDto(
            url = url("stream", "id" to id, "format" to "raw"), headers = emptyMap(), mimeType = s.str("contentType") ?: "audio/mpeg",
            expiresInSeconds = 86_400 * 365, client = config.id, requireBoundedRange = false, rangeChunkSizeBytes = 0, useRangeChunks = false,
            durationSec = s.int("duration"), bitrate = s.int("bitRate")?.times(1000), loudnessLufs = gain?.let { -18.0 - it },
        )
    }

    override suspend fun radio(id: String): List<TrackDto> {
        val similar = runCatching { call("getSimilarSongs2", "id" to id, "count" to "50").obj("similarSongs2").list("song") }.getOrDefault(emptyList())
        return (similar.ifEmpty { call("getRandomSongs", "size" to "50").obj("randomSongs").list("song") }).map(::track)
    }

    override suspend fun like(id: String, on: Boolean) {
        call(if (on) "star" else "unstar", "id" to id)
    }

    override suspend fun lyrics(id: String): String? = try {
        val s = (call("getLyricsBySongId", "id" to id).obj("lyricsList")["structuredLyrics"] as? JsonArray)?.firstOrNull() as? JsonObject
        val synced = (s?.get("synced") as? JsonPrimitive)?.booleanOrNull == true
        (s?.get("line") as? JsonArray)?.mapNotNull { it as? JsonObject }?.joinToString("\n") { l ->
            (if (synced) (l["start"] as? JsonPrimitive)?.longOrNull?.let { "[${lrcTimestamp(it)}]" }.orEmpty() else "") + l.str("value").orEmpty()
        }?.takeIf { it.isNotBlank() }
    } catch (e: CancellationException) {
        throw e
    } catch (_: Exception) {
        null
    }

    private suspend fun write(method: String, vararg params: Pair<String, String>) { call(method, *params) }

    override suspend fun createPlaylist(title: String, trackIds: List<String>): String {
        val p = call("createPlaylist", *(listOf("name" to title) + trackIds.map { "songId" to it }).toTypedArray()).obj("playlist")
        return p.str("id") ?: throw SubsonicError(0, "Navidrome returned no playlist id")
    }

    override suspend fun renamePlaylist(id: String, title: String) = write("updatePlaylist", "playlistId" to id, "name" to title)

    override suspend fun deletePlaylist(id: String) = write("deletePlaylist", "id" to id)

    override suspend fun addToPlaylist(id: String, trackIds: List<String>) =
        write("updatePlaylist", *(listOf("playlistId" to id) + trackIds.map { "songIdToAdd" to it }).toTypedArray())

    override suspend fun removeFromPlaylist(id: String, trackIds: List<String>) {
        // Subsonic removes by position in the playlist.
        val at = call("getPlaylist", "id" to id).obj("playlist").list("entry").withIndex().filter { it.value.str("id") in trackIds }.map { it.index }
        if (at.isNotEmpty()) write("updatePlaylist", *(listOf("playlistId" to id) + at.map { "songIndexToRemove" to it.toString() }).toTypedArray())
    }

    /** Navidrome's own (non-Subsonic) API: log in for a token, then post the image as a form. Playlists only. */
    override suspend fun setImage(kind: String, id: String, bytes: ByteArray, mime: String) {
        check(kind == "playlist") { "Navidrome only lets you change a playlist's image" }
        val login = http.post("$base/auth/login") {
            contentType(ContentType.Application.Json)
            setBody(buildJsonObject { put("username", JsonPrimitive(config.username.orEmpty())); put("password", JsonPrimitive(config.secret.orEmpty())) }.toString())
        }
        val token = runCatching { Json.parseToJsonElement(login.bodyAsText()).jsonObject.str("token") }.getOrNull()
            ?: throw SubsonicError(0, "Navidrome refused the login for the image upload")
        val r = http.post("$base/api/playlist/${enc(id)}/image") {
            header("X-ND-Authorization", "Bearer $token")
            setBody(MultiPartFormDataContent(formData {
                append("image", bytes, Headers.build { append(HttpHeaders.ContentType, mime); append(HttpHeaders.ContentDisposition, "filename=\"cover\"") })
            }))
        }
        if (!r.status.isSuccess()) throw SubsonicError(0, "Navidrome refused the image (HTTP ${r.status.value}); playlist images need a recent Navidrome")
    }

    override suspend fun reportPlay(id: String, state: String, pos: Double) {
        when (state) {
            "start" -> call("scrobble", "id" to id, "submission" to "false")
            "finish" -> call("scrobble", "id" to id, "submission" to "true")
        }
    }

    private companion object {
        /** Subsonic has no starred playlist; this virtual one lists the starred songs. */
        const val STARRED = "starred"
        val STARRED_ITEM = ItemDto("playlist", STARRED, "Starred songs")
        val http = HttpClient(OkHttp) { install(HttpTimeout) { requestTimeoutMillis = 20_000; connectTimeoutMillis = 8_000 } }
        fun JsonObject.str(k: String) = (this[k] as? JsonPrimitive)?.contentOrNull
        fun JsonObject.int(k: String) = (this[k] as? JsonPrimitive)?.intOrNull
    }
}
