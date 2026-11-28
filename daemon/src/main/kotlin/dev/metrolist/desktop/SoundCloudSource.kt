package dev.metrolist.desktop

import io.ktor.client.HttpClient
import io.ktor.client.engine.okhttp.OkHttp
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.plugins.defaultRequest
import io.ktor.client.request.header
import io.ktor.client.request.parameter
import io.ktor.client.request.request
import io.ktor.client.statement.HttpResponse
import io.ktor.client.statement.bodyAsText
import io.ktor.http.HttpMethod
import io.ktor.http.isSuccess
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.longOrNull
import java.io.File

/**
 * SoundCloud through the unofficial api-v2 the web player uses. `config.secret` is the optional `oauth_token` cookie value.
 * Without it search, streaming and public pages work; with it likes, library and feed too.
 */
class SoundCloudSource(override val config: SourceConfig) : MusicSource {
    internal var api = "https://api-v2.soundcloud.com"
    internal var web = "https://soundcloud.com"

    private val token = config.secret?.takeIf { it.isNotBlank() }

    override val kind = Kind.CATALOG
    override val caps = buildSet {
        addAll(listOf(Cap.SEARCH, Cap.HOME, Cap.RADIO))
        if (token != null) addAll(listOf(Cap.LIBRARY, Cap.LIKE))
    }

    @Volatile private var clientId: String? = null
    @Volatile private var meId: String? = null
    private val idLock = Mutex()

    override suspend fun test(): TestResultDto = try {
        if (token != null) get("/me").obj().let { me -> TestResultDto(true, detail = "Signed in as ${me?.str("username") ?: "?"}", avatar = me?.str("avatar_url")) }
        else {
            get("/search/tracks", "q" to "a", "limit" to "1")
            TestResultDto(true, detail = "Not signed in (search and streaming only)")
        }
    } catch (e: CancellationException) {
        throw e
    } catch (e: Exception) {
        TestResultDto(false, error = if (token != null && e.message?.contains("401") == true) "SoundCloud rejected the token: ${e.message}" else e.message ?: "failed")
    }

    override suspend fun search(q: String, type: String): List<ItemDto> {
        val path = when (type) {
            "song" -> "/search/tracks"
            "album" -> "/search/albums"
            "playlist" -> "/search/playlists_without_albums"
            "artist" -> "/search/users"
            else -> return emptyList()
        }
        return items(get(path, "q" to q, "limit" to "30"))
    }

    override suspend fun stream(id: String): StreamDto {
        val t = get("/tracks/$id").obj() ?: error("SoundCloud returned no track")
        val tcs = t.obj("media")?.get("transcodings").list()
        fun proto(p: String) = tcs.filter { it.obj("format")?.str("protocol") == p }
        // Most tracks are HLS-only fMP4 AAC; the daemon stitches those into one rangeable file (Hls). Encrypted variants are skipped.
        val progressive = proto("progressive").firstOrNull()
        val tc = progressive ?: proto("hls").filter { it.obj("format")?.str("mime_type")?.startsWith("audio/mp4") == true }.maxByOrNull { kbps(it.str("preset")) }
            ?: error("This track is not playable (DRM-protected or no supported format)")
        val url = get(tc.str("url") ?: error("transcoding without url"), "track_authorization" to (t.str("track_authorization") ?: "")).obj()?.str("url")
            ?: error("SoundCloud returned no media url")
        val dto = StreamDto(
            url = url, headers = emptyMap(), mimeType = tc.obj("format")?.str("mime_type") ?: "audio/mpeg", expiresInSeconds = 1800, client = config.id,
            requireBoundedRange = false, rangeChunkSizeBytes = 0, useRangeChunks = false, durationSec = t.long("duration")?.div(1000)?.toInt(),
            bitrate = kbps(tc.str("preset")).takeIf { it > 0 }?.times(1000),
        )
        return if (progressive != null) dto else Hls.stream(url, dto)
    }

    /** `aac_160k` -> 160. */
    private fun kbps(preset: String?) = preset?.let { Regex("""(\d+)k""").find(it)?.groupValues?.get(1)?.toInt() } ?: 0

    override suspend fun home(): List<SectionDto> {
        val out = mutableListOf<SectionDto>()
        suspend fun add(title: String, shelf: String, path: String, vararg p: Pair<String, String>) {
            runCatching { items(get(path, *p)) }.getOrNull()?.takeIf { it.isNotEmpty() }?.let { out += SectionDto(title, it, shelf) }
        }
        if (token != null) {
            add("Your feed", "recent", "/stream", "limit" to "30")
            runCatching { add("Your likes", "recent", "/users/${me()}/track_likes", "limit" to "20") }
        }
        runCatching {
            // ponytail: api-v2 /charts answers {} without a browser session, so curated selections are the only anonymous home.
            collection(get("/mixed-selections", "limit" to "10")).take(8).forEach { s ->
                val its = items(s.obj("items") ?: return@forEach)
                if (its.isNotEmpty()) out += SectionDto(s.str("title") ?: "Selection", its, "discover")
            }
        }
        return out
    }

    override suspend fun library(kind: String): List<ItemDto> {
        if (token == null) return emptyList()
        val me = me()
        // ponytail: first page only (limit 200); follow next_href if libraries get bigger.
        fun page(path: String) = suspend { items(get("/users/$me/$path", "limit" to "200")) }
        return when (kind) {
            "songs" -> page("track_likes")()
            "artists" -> page("followings")()
            "playlists" -> listOf(FAVORITES_ITEM) + page("playlists_without_albums")() + page("playlist_likes")().filter { it.type == "playlist" }
            "albums" -> page("albums")() + page("playlist_likes")().filter { it.type == "album" }
            else -> emptyList()
        }.distinctBy { it.id }
    }

    override suspend fun album(id: String) = if (id.startsWith(SINGLE)) single(id.removePrefix(SINGLE)) else set(id)
    override suspend fun playlist(id: String) = if (id == FAVORITES) favorites() else set(id)

    /** The sidebar's "Liked songs", like Jellyfin's. ponytail: first 200 likes; follow next_href for more. */
    private suspend fun favorites() =
        PageDto(item = FAVORITES_ITEM, tracks = items(get("/users/${me()}/track_likes", "limit" to "200")).mapNotNull(::track))

    override suspend fun artist(id: String): PageDto {
        val u = get("/users/$id").obj() ?: error("SoundCloud returned no user")
        // The client only draws an artist's shelves when `tracks` is empty, and renders a "Top songs" shelf as a track list, like YouTube and Jellyfin.
        val top = runCatching { items(get("/users/$id/toptracks", "limit" to "10")) }.getOrNull()?.takeIf { it.isNotEmpty() }
            ?: items(get("/users/$id/tracks", "limit" to "10"))
        return PageDto(
            item = item(u) ?: error("bad user"), description = u.str("description"),
            sections = listOf(
                SectionDto("Top songs", top), SectionDto("Albums", items(get("/users/$id/albums", "limit" to "20"))),
                SectionDto("Playlists", items(get("/users/$id/playlists_without_albums", "limit" to "20"))),
            ).filter { it.items.isNotEmpty() },
        )
    }

    /**
     * Every album, the artist's own playlists and every upload that is in neither (shown as one-track "singles"), newest first.
     * ponytail: capped at 20 pages of 50 per list.
     */
    override suspend fun discography(id: String): List<PageDto> = coroutineScope {
        val name = get("/users/$id").obj()?.str("username")
        fun dated(o: JsonObject) = listOfNotNull(name, (o.str("release_date") ?: o.str("created_at"))?.take(4)).joinToString(" · ")
        val gate = Semaphore(4)
        val sets = (pages("/users/$id/albums") + pages("/users/$id/playlists_without_albums")).distinctBy(::idOf)
            .map { s -> async { gate.withPermit { runCatching { set(idOf(s)!!).let { it.copy(item = it.item.copy(subtitle = dated(s))) } }.getOrNull() } } }
            .awaitAll().filterNotNull().filter { p -> p.item.type == "album" || p.tracks.all { it.artistId == id } }
        val inSets = sets.flatMapTo(HashSet()) { p -> p.tracks.map { it.id } }
        val singles = pages("/users/$id/tracks").filter { idOf(it) !in inSets }.mapNotNull { t ->
            val i = item(t) ?: return@mapNotNull null
            PageDto(i.copy(type = "album", id = SINGLE + i.id, subtitle = dated(t)), tracks = listOfNotNull(track(i)))
        }
        (sets + singles).sortedByDescending { it.item.subtitle?.substringAfterLast(" · ")?.takeIf { y -> y.length == 4 }?.toIntOrNull() ?: 0 }
    }

    /** Follows `next_href` so long catalogues are complete. */
    private suspend fun pages(path: String): List<JsonObject> {
        val out = mutableListOf<JsonObject>()
        var r = get(path, "limit" to "50")
        repeat(20) {
            out += collection(r)
            r = get((r as? JsonObject)?.str("next_href") ?: return out)
        }
        return out
    }

    /** A track released on its own, opened as a one-song album. */
    private suspend fun single(id: String): PageDto {
        val i = item(get("/tracks/$id").obj() ?: error("SoundCloud returned no track")) ?: error("bad track")
        return PageDto(i.copy(type = "album", id = SINGLE + id), tracks = listOfNotNull(track(i)))
    }

    override suspend fun radio(id: String): List<TrackDto> = items(get("/tracks/$id/related", "limit" to "30")).mapNotNull(::track)

    override suspend fun like(id: String, on: Boolean) {
        if (token == null) unsupported()
        // DataDome, SoundCloud's bot protection, always answers plain HTTP clients with a captcha on likes, so go straight to a real browser session when there is one.
        System.getenv("SERAFIN_LOGIN_HELPER")?.takeIf { File(it).isFile }?.let { return likeInBrowser(it, id, on) }
        val r = call(if (on) HttpMethod.Put else HttpMethod.Delete, "/users/${me()}/track_likes/$id", emptyMap())
        check(r.status.isSuccess()) { "SoundCloud like failed: HTTP ${r.status.value} ${r.bodyAsText().take(120)}" }
    }

    /** Sends the like from the login helper's webview (its own SoundCloud session), which shows the captcha window if DataDome insists. */
    private suspend fun likeInBrowser(helper: String, id: String, on: Boolean) {
        val (user, client) = me() to clientId()
        val (code, out) = withContext(Dispatchers.IO) {
            val p = ProcessBuilder(helper, "soundcloud-like", if (on) "on" else "off", user, id).redirectErrorStream(true)
                .apply { environment().also { it["SC_TOKEN"] = token; it["SC_CLIENT_ID"] = client } }.start()
            val text = p.inputStream.bufferedReader().readText()
            p.waitFor() to text.trim()
        }
        check(code == 0) { "SoundCloud like failed: ${out.take(200)}" }
    }

    /** A set with its tracks; past the first few SoundCloud returns id-only stubs, filled in via /tracks?ids=. */
    private suspend fun set(id: String): PageDto {
        val p = get(if (':' in id) "/system-playlists/$id" else "/playlists/$id").obj() ?: error("SoundCloud returned no playlist")
        val raw = p["tracks"].list()
        val full = HashMap<String, JsonObject>()
        raw.filter { it.str("title") == null }.mapNotNull(::idOf).chunked(50).forEach { ids ->
            get("/tracks", "ids" to ids.joinToString(",")).list().forEach { o -> idOf(o)?.let { full[it] = o } }
        }
        val tracks = raw.mapNotNull { item(full[idOf(it)] ?: it) }.mapNotNull(::track)
        val user = p.obj("user")
        val item = (item(p) ?: error("bad playlist")).copy(author = user?.str("username"), authorAvatar = user?.str("avatar_url")?.replace("-large", "-t300x300"))
        return PageDto(item = item, description = p.str("description"), tracks = tracks)
    }

    private suspend fun me(): String = meId ?: (get("/me").obj()?.let(::idOf) ?: error("SoundCloud returned no user")).also { meId = it }

    private fun track(i: ItemDto) = if (i.type != "song") null else
        TrackDto(i.id, i.title, listOfNotNull(i.subtitle), durationSec = i.durationSec, thumbnail = i.thumbnail, source = config.id, artistId = i.artistId)

    private fun items(e: JsonElement) = collection(e).mapNotNull(::item)

    private fun collection(e: JsonElement) = (e as? JsonObject)?.get("collection").list()

    /** Maps a track, set or user, unwrapping the `{track|playlist: ...}` envelopes of feeds and likes. */
    private fun item(o: JsonObject): ItemDto? {
        val e = o.obj("track") ?: o.obj("playlist") ?: o
        val id = idOf(e) ?: return null
        val user = e.obj("user")
        return when (e.str("kind")) {
            "track" -> ItemDto("song", id, e.str("title") ?: return null, user?.str("username"), thumb(e), e.long("duration")?.div(1000)?.toInt(), artistId = user?.let(::idOf))
            "user" -> ItemDto("artist", id, e.str("username") ?: return null, thumbnail = thumb(e))
            "playlist", "system-playlist" ->
                ItemDto(if (e.bool("is_album")) "album" else "playlist", id, e.str("title") ?: return null, user?.str("username"), thumb(e) ?: e["tracks"].list().firstNotNullOfOrNull(::thumb), artistId = user?.let(::idOf))
            else -> null
        }
    }

    private fun thumb(e: JsonObject) = (e.str("artwork_url") ?: e.obj("user")?.str("avatar_url") ?: e.str("avatar_url"))?.replace("-large", "-t300x300")

    private suspend fun get(path: String, vararg params: Pair<String, String>): JsonElement {
        val r = call(HttpMethod.Get, path, params.toMap())
        check(r.status.isSuccess()) { "SoundCloud HTTP ${r.status.value} for ${path.substringBefore('?').substringAfter("://")}" }
        return Json.parseToJsonElement(r.bodyAsText())
    }

    /** Adds client_id and the token; on 401/403 re-discovers the client_id once and retries. */
    private suspend fun call(method: HttpMethod, path: String, params: Map<String, String>): HttpResponse {
        suspend fun once() = http.request(if (path.startsWith("http")) path else api + path) {
            this.method = method
            parameter("client_id", clientId())
            params.forEach { (k, v) -> parameter(k, v) }
            token?.let { header("Authorization", "OAuth $it") }
        }
        val r = once()
        if (r.status.value != 401 && r.status.value != 403) return r
        val stale = clientId
        idLock.withLock { if (clientId == stale) clientId = null }
        return once()
    }

    private suspend fun clientId(): String = clientId ?: idLock.withLock { clientId ?: discoverClientId().also { clientId = it } }

    // ponytail: the fragile part. SoundCloud has no public API for new apps, so this scrapes the client_id out of the web player's
    // script bundles. If it stops working, fix the bundle regex or the `client_id:"..."` pattern here and nowhere else.
    private suspend fun discoverClientId(): String {
        val page = http.request(web) {}.bodyAsText()
        for (b in Regex("""https?://[^"'\s]+/assets/[^"'\s]+\.js""").findAll(page).map { it.value }.toList().reversed()) {
            Regex("""client_id\s*:\s*"([A-Za-z0-9]{16,})"""").find(http.request(b) {}.bodyAsText())?.let { return it.groupValues[1] }
        }
        error("could not find a SoundCloud client_id")
    }

    private companion object {
        const val FAVORITES = "favorites"
        const val SINGLE = "t-"
        val FAVORITES_ITEM = ItemDto("playlist", FAVORITES, "Liked songs")
        val http = HttpClient(OkHttp) {
            install(HttpTimeout) { connectTimeoutMillis = 15_000; requestTimeoutMillis = 30_000; socketTimeoutMillis = 30_000 }
            defaultRequest { header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0") }
        }
    }
}

private fun JsonElement?.obj() = this as? JsonObject
private fun JsonElement?.list() = (this as? JsonArray)?.mapNotNull { it as? JsonObject } ?: emptyList()
private fun JsonObject.obj(k: String) = this[k] as? JsonObject
private fun JsonObject.str(k: String) = (this[k] as? JsonPrimitive)?.contentOrNull
private fun JsonObject.long(k: String) = (this[k] as? JsonPrimitive)?.longOrNull
private fun JsonObject.bool(k: String) = (this[k] as? JsonPrimitive)?.booleanOrNull == true
private fun idOf(o: JsonObject) = o.str("id") ?: o.str("urn")
