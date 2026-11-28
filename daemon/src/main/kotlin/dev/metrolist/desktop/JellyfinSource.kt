package dev.metrolist.desktop

import io.ktor.client.HttpClient
import io.ktor.client.engine.okhttp.OkHttp
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.request.header
import io.ktor.client.request.request
import io.ktor.client.request.setBody
import io.ktor.client.statement.bodyAsText
import io.ktor.http.ContentType
import io.ktor.http.HttpMethod
import io.ktor.http.HttpStatusCode
import io.ktor.http.contentType
import io.ktor.http.isSuccess
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put
import java.net.URLEncoder
import java.util.Base64
import java.util.concurrent.CancellationException

class JellyfinSource(override val config: SourceConfig) : MusicSource {
    override val kind = Kind.LIBRARY
    override val caps = setOf(Cap.SEARCH, Cap.HOME, Cap.LIBRARY, Cap.LIKE, Cap.RADIO, Cap.SCROBBLE, Cap.PLAYLIST_WRITE, Cap.IMAGE, Cap.SYNCPLAY)

    private class Failure(message: String) : Exception(message)

    private val base = config.url.orEmpty().trim().trimEnd('/')
    private val deviceId = "serafin-${config.id}"
    private val authMutex = Mutex()
    @Volatile private var token: String? = null
    @Volatile private var userId: String? = null

    private val syncPlayLazy = lazy {
        SyncPlay(config.id, base, deviceId, { authenticate(false, null); token!! }, { stale -> authenticate(true, stale); token!! }) { m, p, b -> call(m, p, body = b) }
    }
    val syncPlay by syncPlayLazy

    /** Stops the SyncPlay session if one was ever created; called when this instance is dropped. */
    fun shutdown() { if (syncPlayLazy.isInitialized()) syncPlay.shutdown() }

    /** Tracks for `<src>:`-prefixed ids in one call, in the order asked; unknown ids are skipped. */
    suspend fun tracksByIds(ids: List<String>): List<TrackDto> {
        val raw = ids.take(MAX_IDS).map { it.removePrefix("${config.id}:") }
        // Batches keep the URL under Kestrel's default 8 KB request line.
        val byId = raw.chunked(100).flatMap { tracks(list("/Items", mapOf("userId" to uid(), "Ids" to it.joinToString(",")))) }.associateBy { it.id }
        return raw.mapNotNull { byId[it] }
    }

    private fun authHeader(tok: String? = null) =
        "MediaBrowser Client=\"Serafin\", Device=\"Desktop\", DeviceId=\"$deviceId\", Version=\"1\"" + (tok?.let { ", Token=\"$it\"" } ?: "")

    private fun enc(s: String) = URLEncoder.encode(s, Charsets.UTF_8)
    private fun query(p: Map<String, String>) = if (p.isEmpty()) "" else p.entries.joinToString("&", "?") { "${it.key}=${enc(it.value)}" }

    private suspend fun send(method: HttpMethod, path: String, p: Map<String, String>, body: JsonObject?, tok: String?) =
        http.request(base + path + query(p)) {
            this.method = method
            header("Authorization", authHeader(tok))
            tok?.let { header("X-Emby-Token", it) }
            if (body != null) { contentType(ContentType.Application.Json); setBody(body.toString()) }
        }

    private suspend fun authenticate(force: Boolean, stale: String?) = authMutex.withLock {
        // a concurrent caller may already have refreshed the token that this caller saw rejected
        if (token != null && (!force || token != stale)) return@withLock
        val r = send(HttpMethod.Post, "/Users/AuthenticateByName", emptyMap(), buildJsonObject {
            put("Username", config.username.orEmpty()); put("Pw", config.secret.orEmpty())
        }, null)
        if (r.status == HttpStatusCode.Unauthorized || r.status == HttpStatusCode.BadRequest) throw Failure("Wrong username or password")
        if (!r.status.isSuccess()) throw Failure("Jellyfin login failed (HTTP ${r.status.value})")
        val o = parse(r.bodyAsText()) as? JsonObject
        token = o?.str("AccessToken") ?: throw Failure("Jellyfin returned no access token")
        userId = o.obj("User")?.str("Id") ?: throw Failure("Jellyfin returned no user")
    }

    /** Authenticated call; re-authenticates once on 401. Returns the parsed body (null when empty). */
    private suspend fun call(method: HttpMethod, path: String, p: Map<String, String> = emptyMap(), body: JsonObject? = null): JsonElement? {
        if (base.isEmpty()) throw Failure("No server URL set")
        authenticate(false, null)
        var used = token
        var r = send(method, path, p, body, used)
        if (r.status == HttpStatusCode.Unauthorized) {
            authenticate(true, used)
            used = token
            r = send(method, path, p, body, used)
        }
        if (!r.status.isSuccess()) throw Failure(refusal(r.status.value))
        return parse(r.bodyAsText())
    }

    private fun refusal(code: Int) = when (code) {
        403 -> "Your Jellyfin account isn't allowed to do that"
        404 -> "Jellyfin doesn't have that item, or this server is too old to support it (10.9 or newer is needed for playlist edits)"
        else -> "Jellyfin request failed (HTTP $code)"
    }

    /** Like [call] with a raw text body, for the image upload (Jellyfin wants the image base64-encoded). */
    private suspend fun callRaw(method: HttpMethod, path: String, body: String, mime: String) {
        authenticate(false, null)
        suspend fun go(tok: String?) = http.request(base + path) {
            this.method = method
            header("Authorization", authHeader(tok)); tok?.let { header("X-Emby-Token", it) }
            contentType(ContentType.parse(mime)); setBody(body)
        }
        var used = token
        var r = go(used)
        if (r.status == HttpStatusCode.Unauthorized) { authenticate(true, used); used = token; r = go(used) }
        if (!r.status.isSuccess()) throw Failure(refusal(r.status.value))
    }

    private suspend fun list(path: String, p: Map<String, String> = emptyMap()): List<JsonObject> {
        val e = call(HttpMethod.Get, path, p)
        val arr = (e as? JsonArray) ?: (e as? JsonObject)?.get("Items") as? JsonArray
        return arr.orEmpty().filterIsInstance<JsonObject>()
    }

    private suspend fun uid(): String { authenticate(false, null); return userId!! }

    override suspend fun test(): TestResultDto = try {
        if (base.isEmpty()) throw Failure("No server URL set")
        val r = send(HttpMethod.Get, "/System/Info/Public", emptyMap(), null, null)
        val info = if (r.status.isSuccess()) parse(r.bodyAsText()) as? JsonObject else null
        val version = info?.str("Version")
        if (info == null || version == null || info.str("ProductName")?.contains("Jellyfin", true) == false) throw Failure("Not a Jellyfin server")
        token = null
        authenticate(false, null)
        TestResultDto(true, detail = "Jellyfin $version as ${config.username}")
    } catch (e: CancellationException) {
        throw e
    } catch (e: Failure) {
        TestResultDto(false, error = e.message)
    } catch (e: java.net.UnknownHostException) {
        TestResultDto(false, error = "Server not found")
    } catch (e: java.net.ConnectException) {
        TestResultDto(false, error = "Connection refused")
    } catch (e: java.net.SocketTimeoutException) {
        TestResultDto(false, error = "Server timed out")
    } catch (e: Exception) {
        TestResultDto(false, error = "Could not reach the server, check the URL")
    }

    private fun thumb(o: JsonObject): String? {
        val id = if (o.obj("ImageTags")?.get("Primary") != null) o.str("Id") else if (o.str("Type") == "Audio") o.str("AlbumId") else null
        val tag = o.obj("ImageTags")?.str("Primary")?.takeIf { id == o.str("Id") }?.let { "&tag=$it" }.orEmpty() // changes when the image does
        // ponytail: images need no auth on Jellyfin, so the URL stays token-free and can go to Discord; a server that gates them needs api_key back
        return id?.let { "$base/Items/$it/Images/Primary?maxWidth=400$tag" }
    }

    private fun seconds(o: JsonObject) = o.long("RunTimeTicks")?.let { (it / TICKS_PER_SEC).toInt() }
    private fun artistName(o: JsonObject) = o.arr("Artists")?.firstOrNull()?.str() ?: o.str("AlbumArtist")
    private fun artistId(o: JsonObject) = o.arr("ArtistItems")?.firstOrNull()?.jsonObject?.str("Id") ?: o.arr("AlbumArtists")?.firstOrNull()?.jsonObject?.str("Id")

    private fun item(o: JsonObject): ItemDto? {
        val id = o.str("Id") ?: return null
        val type = when (o.str("Type")) { "Audio" -> "song"; "MusicAlbum" -> "album"; "MusicArtist" -> "artist"; "Playlist" -> "playlist"; else -> return null }
        return ItemDto(
            type = type, id = id, title = o.str("Name").orEmpty(),
            subtitle = when (type) {
                "artist", "playlist" -> null
                "album" -> listOfNotNull(o.str("AlbumArtist") ?: artistName(o), o.int("ProductionYear")?.toString()).joinToString(" · ").ifEmpty { null }
                else -> artistName(o)
            },
            thumbnail = thumb(o), durationSec = if (type == "song") seconds(o) else null,
            artistId = when (type) {
                "song" -> artistId(o)
                "album" -> o.arr("AlbumArtists")?.firstOrNull()?.jsonObject?.str("Id") ?: artistId(o)
                else -> null
            },
            album = if (type == "song") o.str("Album") else null,
            albumId = if (type == "song") o.str("AlbumId") else null,
            editable = type == "playlist" && (o["CanDelete"] as? JsonPrimitive)?.booleanOrNull == true,
        )
    }

    private fun track(o: JsonObject): TrackDto? {
        val id = o.str("Id") ?: return null
        return TrackDto(
            id = id, title = o.str("Name").orEmpty(),
            artists = o.arr("Artists")?.mapNotNull { it.str() }?.takeIf { it.isNotEmpty() } ?: listOfNotNull(o.str("AlbumArtist")),
            album = o.str("Album"), durationSec = seconds(o), thumbnail = thumb(o), source = config.id,
            artistId = artistId(o), albumId = o.str("AlbumId"),
            liked = o.obj("UserData")?.get("IsFavorite").prim()?.booleanOrNull == true,
        )
    }

    private fun items(l: List<JsonObject>) = l.mapNotNull(::item)
    private fun tracks(l: List<JsonObject>) = l.mapNotNull(::track)

    override suspend fun search(q: String, type: String): List<ItemDto> {
        val s = mapOf("searchTerm" to q, "Limit" to "30", "userId" to uid())
        val l = when (type) {
            "song" -> list("/Items", s + mapOf("IncludeItemTypes" to "Audio", "Recursive" to "true"))
            "album" -> list("/Items", s + mapOf("IncludeItemTypes" to "MusicAlbum", "Recursive" to "true"))
            "artist" -> list("/Artists", s)
            "playlist" -> list("/Items", s + mapOf("IncludeItemTypes" to "Playlist", "Recursive" to "true"))
            else -> emptyList()
        }
        return items(l)
    }

    override suspend fun library(kind: String): List<ItemDto> {
        val u = mapOf("userId" to uid(), "Recursive" to "true", "SortBy" to "SortName")
        // ponytail: fixed caps, no paging; libraries beyond the cap are truncated until the client pages
        return items(when (kind) {
            "songs" -> list("/Items", u + mapOf("IncludeItemTypes" to "Audio", "Filters" to "IsFavorite", "Limit" to "1000"))
            "albums" -> list("/Items", u + mapOf("IncludeItemTypes" to "MusicAlbum", "Limit" to "500"))
            "artists" -> list("/Artists/AlbumArtists", mapOf("userId" to u.getValue("userId"), "SortBy" to "SortName", "Limit" to "500"))
            "playlists" -> list("/Items", u + mapOf("IncludeItemTypes" to "Playlist", "Limit" to "200", "Fields" to "CanDelete"))
            else -> emptyList()
        }).let { if (kind == "playlists") listOf(FAVORITES_ITEM) + it else it }
    }

    override suspend fun home(): List<SectionDto> {
        val u = uid()
        val songs = mapOf("userId" to u, "IncludeItemTypes" to "Audio", "Recursive" to "true", "Limit" to "20")
        // Same queries as jellyfin-web's music views. /UserItems/Resume is for half-heard long tracks, not recently played.
        val cont = items(list("/Items", songs + mapOf("SortBy" to "DatePlayed", "SortOrder" to "Descending", "Filters" to "IsPlayed")))
        // Latest groups new tracks by album, ordered by when the tracks were added; asking for MusicAlbum sorts by folder dates instead.
        val recent = items(list("/Items/Latest", mapOf("userId" to u, "IncludeItemTypes" to "Audio", "Limit" to "20")))
        val playedRaw = list("/Items", songs + mapOf("SortBy" to "PlayCount", "SortOrder" to "Descending", "Filters" to "IsPlayed"))
        val seed = list("/Items", songs + mapOf("Filters" to "IsFavorite", "Limit" to "1")).firstOrNull() ?: playedRaw.firstOrNull()
        val mixes = seed?.str("Id")?.let { id -> items(mix(id)) }.orEmpty()
        return listOf(
            SectionDto("Continue listening", cont, "continue"),
            SectionDto("Recently added", recent, "recent"),
            SectionDto("Most played", items(playedRaw), "played"),
            SectionDto("Mix from ${seed?.str("Name").orEmpty()}", mixes, "mixes"),
        ).filter { it.items.isNotEmpty() }
    }

    private suspend fun mix(id: String) = list("/Items/${enc(id)}/InstantMix", mapOf("userId" to uid(), "Limit" to "50"))

    override suspend fun radio(id: String) = tracks(mix(id))

    private suspend fun detail(id: String): JsonObject =
        call(HttpMethod.Get, "/Items/${enc(id)}", mapOf("userId" to uid(), "Fields" to "CanDelete,Overview")) as? JsonObject ?: throw Failure("Item not found")

    override suspend fun album(id: String): PageDto {
        val t = list("/Items", mapOf("userId" to uid(), "ParentId" to id, "IncludeItemTypes" to "Audio", "Recursive" to "true", "SortBy" to "ParentIndexNumber,IndexNumber,SortName"))
        return PageDto(item(detail(id)) ?: throw Failure("Not an album"), tracks = tracks(t))
    }

    override suspend fun playlist(id: String): PageDto {
        if (id == FAVORITES) return PageDto(FAVORITES_ITEM, tracks = tracks(list("/Items", mapOf("userId" to uid(), "IncludeItemTypes" to "Audio", "Recursive" to "true", "Filters" to "IsFavorite", "SortBy" to "SortName"))))
        val t = list("/Playlists/${enc(id)}/Items", mapOf("userId" to uid()))
        return PageDto(item(detail(id)) ?: throw Failure("Not a playlist"), tracks = tracks(t))
    }

    // Shelves only, like a YouTube artist page: the client shows a page with tracks as a bare track list.
    // Albums / Appears on use jellyfin-web's own artist queries.
    override suspend fun artist(id: String): PageDto {
        val u = uid()
        val a = detail(id)
        val base = mapOf("userId" to u, "Recursive" to "true")
        val newest = mapOf("SortBy" to "PremiereDate,ProductionYear,SortName", "SortOrder" to "Descending")
        val top = list("/Items", base + mapOf("ArtistIds" to id, "IncludeItemTypes" to "Audio", "SortBy" to "PlayCount,SortName", "SortOrder" to "Descending,Ascending", "Limit" to "10"))
        val albums = list("/Items", base + newest + mapOf("AlbumArtistIds" to id, "IncludeItemTypes" to "MusicAlbum"))
        val appears = list("/Items", base + newest + mapOf("ContributingArtistIds" to id, "IncludeItemTypes" to "MusicAlbum"))
        val similar = runCatching { list("/Artists/${enc(id)}/Similar", mapOf("userId" to u, "limit" to "12")) }.getOrDefault(emptyList())
        return PageDto(
            item(a) ?: throw Failure("Not an artist"), description = a.str("Overview"),
            sections = listOf(
                SectionDto("Top songs", items(top)), SectionDto("Albums", items(albums)),
                SectionDto("Appears on", items(appears)), SectionDto("Similar artists", items(similar)),
            ).filter { it.items.isNotEmpty() },
        )
    }

    override suspend fun stream(id: String): StreamDto {
        val o = detail(id)
        val src = o.arr("MediaSources")?.firstOrNull() as? JsonObject
        val container = src?.str("Container")?.lowercase().orEmpty()
        val mime = when {
            "flac" in container -> "audio/flac"
            "mp3" in container -> "audio/mpeg"
            "ogg" in container || "opus" in container -> "audio/ogg"
            "wav" in container -> "audio/wav"
            "aac" in container -> "audio/aac"
            "m4a" in container || "mp4" in container || "mov" in container -> "audio/mp4"
            else -> "audio/mpeg"
        }
        return StreamDto(
            // ponytail: api_key in the URL (the player can't set headers); token is exposed to anything that sees the URL
            url = "$base/Audio/${enc(id)}/stream?static=true&api_key=$token",
            headers = emptyMap(), mimeType = mime, expiresInSeconds = 86_400 * 30, client = config.id,
            requireBoundedRange = false, rangeChunkSizeBytes = 0, useRangeChunks = false,
            durationSec = seconds(o), bitrate = src?.int("Bitrate"),
            // NormalizationGain is the dB needed to reach -18 LUFS
            loudnessLufs = o.dbl("NormalizationGain")?.let { -18.0 - it },
        )
    }

    override suspend fun like(id: String, on: Boolean) {
        call(if (on) HttpMethod.Post else HttpMethod.Delete, "/UserFavoriteItems/${enc(id)}", mapOf("userId" to uid()))
    }

    override suspend fun createPlaylist(title: String, trackIds: List<String>): String {
        val o = call(HttpMethod.Post, "/Playlists", body = buildJsonObject {
            put("Name", title); put("UserId", uid()); put("MediaType", "Audio")
            put("Ids", JsonArray(trackIds.map(::JsonPrimitive)))
        }) as? JsonObject
        return o?.str("Id") ?: throw Failure("Jellyfin returned no playlist id")
    }

    override suspend fun renamePlaylist(id: String, title: String) {
        call(HttpMethod.Post, "/Playlists/${enc(id)}", body = buildJsonObject { put("Name", title) })
    }

    override suspend fun deletePlaylist(id: String) {
        call(HttpMethod.Delete, "/Items/${enc(id)}")
    }

    override suspend fun addToPlaylist(id: String, trackIds: List<String>) {
        call(HttpMethod.Post, "/Playlists/${enc(id)}/Items", mapOf("ids" to trackIds.joinToString(","), "userId" to uid()))
    }

    override suspend fun removeFromPlaylist(id: String, trackIds: List<String>) {
        // Jellyfin removes by playlist entry id (the same song can be in a playlist twice), which the playlist's items carry.
        val entries = list("/Playlists/${enc(id)}/Items", mapOf("userId" to uid())).filter { it.str("Id") in trackIds }.mapNotNull { it.str("PlaylistItemId") }
        if (entries.isNotEmpty()) call(HttpMethod.Delete, "/Playlists/${enc(id)}/Items", mapOf("entryIds" to entries.joinToString(",")))
    }

    override suspend fun setImage(kind: String, id: String, bytes: ByteArray, mime: String) {
        callRaw(HttpMethod.Post, "/Items/${enc(id)}/Images/Primary", Base64.getEncoder().encodeToString(bytes), mime)
    }

    override suspend fun lyrics(id: String): String? = try {
        (call(HttpMethod.Get, "/Items/${enc(id)}/Lyrics") as? JsonObject)?.arr("Lyrics")?.mapNotNull { it as? JsonObject }?.joinToString("\n") { l ->
            l.long("Start")?.let { "[${lrcTimestamp(it / 10_000)}]" }.orEmpty() + l.str("Text").orEmpty()
        }?.takeIf { it.isNotBlank() }
    } catch (e: CancellationException) {
        throw e
    } catch (_: Exception) {
        null
    }

    override suspend fun reportPlay(id: String, state: String, pos: Double) {
        // Jellyfin counts the play (PlayCount, DatePlayed) at start; "finish" is our scrobble point, and the stop on leaving the song ends it.
        when (state) {
            "start" -> call(HttpMethod.Post, "/Sessions/Playing", body = buildJsonObject { put("ItemId", id); put("CanSeek", true); put("PlayMethod", "DirectPlay") })
            "finish" -> {}
            else -> call(HttpMethod.Post, "/Sessions/Playing/Stopped", body = buildJsonObject { put("ItemId", id); put("PositionTicks", (pos * TICKS_PER_SEC).toLong()) })
        }
    }

    internal companion object {
        private const val MAX_IDS = 500
        /** Jellyfin has no favorites playlist; this virtual one lists the favorited songs. Real ids are 32 hex chars, so no clash. */
        const val FAVORITES = "favorites"
        val FAVORITES_ITEM = ItemDto("playlist", FAVORITES, "Favorites")
        const val TICKS_PER_SEC = 10_000_000L
        val json = Json { ignoreUnknownKeys = true }
        val http = HttpClient(OkHttp) {
            install(HttpTimeout) { connectTimeoutMillis = 10_000; requestTimeoutMillis = 30_000; socketTimeoutMillis = 30_000 }
            expectSuccess = false
        }

        fun parse(s: String): JsonElement? = if (s.isBlank()) null else runCatching { json.parseToJsonElement(s) }.getOrNull()
        fun JsonElement?.prim() = this as? JsonPrimitive
        fun JsonElement?.str() = prim()?.contentOrNull
        fun JsonObject.str(k: String) = get(k).str()
        fun JsonObject.int(k: String) = get(k).prim()?.intOrNull
        fun JsonObject.long(k: String) = get(k).prim()?.longOrNull
        fun JsonObject.dbl(k: String) = get(k).prim()?.doubleOrNull
        fun JsonObject.obj(k: String) = get(k) as? JsonObject
        fun JsonObject.arr(k: String) = get(k) as? JsonArray
    }
}
