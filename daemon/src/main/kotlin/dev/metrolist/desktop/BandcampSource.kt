package dev.metrolist.desktop

import io.ktor.client.HttpClient
import io.ktor.client.engine.okhttp.OkHttp
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.plugins.defaultRequest
import io.ktor.client.request.header
import io.ktor.client.request.request
import io.ktor.client.request.setBody
import io.ktor.client.statement.bodyAsText
import io.ktor.http.ContentType
import io.ktor.http.HttpMethod
import io.ktor.http.contentType
import io.ktor.http.isSuccess
import kotlinx.coroutines.CancellationException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.put
import java.net.URI
import java.util.Base64

/**
 * Bandcamp through its public web endpoints (no key, no account). Raw ids are the base64url of the track, album or artist page URL,
 * so lookups need no state. Streams are the 128 kbps progressive mp3 Bandcamp embeds in every public page.
 */
class BandcampSource(override val config: SourceConfig = SourceConfig(ID, "bandcamp", NAME)) : MusicSource {
    internal var base = "https://bandcamp.com"

    override val kind = Kind.CATALOG
    override val caps = setOf(Cap.SEARCH, Cap.HOME)

    override suspend fun test(): TestResultDto = try {
        search("a", "song")
        TestResultDto(true, detail = "Public catalogue (search and streaming only)")
    } catch (e: CancellationException) {
        throw e
    } catch (e: Exception) {
        TestResultDto(false, error = e.message ?: "failed")
    }

    override suspend fun search(q: String, type: String): List<ItemDto> {
        val filter = when (type) { "song" -> "t"; "album" -> "a"; "artist" -> "b"; else -> return emptyList() }
        val body = buildJsonObject { put("search_text", q); put("search_filter", filter); put("full_page", false); put("fan_id", JsonNull) }
        val r = Json.parseToJsonElement(send(HttpMethod.Post, "$base/api/bcsearch_public_api/1/autocomplete_elastic", body.toString()))
        return (r as? JsonObject)?.obj("auto")?.get("results").list().mapNotNull { o ->
            val art = o.long("art_id")?.let(::art)
            when (o.str("type")) {
                "t" -> ItemDto("song", enc(o.str("item_url_path") ?: return@mapNotNull null), o.str("name") ?: return@mapNotNull null, o.str("band_name"), art,
                    artistId = o.str("item_url_root")?.let(::enc), album = o.str("album_name"))
                "a" -> ItemDto("album", enc(o.str("item_url_path") ?: return@mapNotNull null), o.str("name") ?: return@mapNotNull null, o.str("band_name"), art, artistId = o.str("item_url_root")?.let(::enc))
                "b" -> ItemDto("artist", enc(o.str("item_url_root") ?: return@mapNotNull null), o.str("name") ?: return@mapNotNull null, o.str("genre_name"), art)
                else -> null
            }
        }
    }

    override suspend fun stream(id: String): StreamDto {
        val page = tralbum(dec(id))
        val t = page["trackinfo"].list().firstOrNull() ?: error("Bandcamp page has no track")
        val url = t.obj("file")?.str("mp3-128")
        if (url == null || page.bool("is_preorder") || t.bool("unreleased_track"))
            error("\"${t.str("title") ?: "This track"}\" is not streamable on Bandcamp (unreleased, pre-order or not available for streaming)")
        return StreamDto(
            url = url, headers = emptyMap(), mimeType = "audio/mpeg", expiresInSeconds = 3600, client = config.id,
            requireBoundedRange = false, rangeChunkSizeBytes = 0, useRangeChunks = false,
            durationSec = t.double("duration")?.toInt(), bitrate = 128_000,
        )
    }

    override suspend fun home(): List<SectionDto> = listOf("Popular" to "top", "New releases" to "new").mapNotNull { (title, sort) ->
        runCatching {
            // ponytail: first page only (p=0); the endpoint paginates with p=1,2,...
            val r = Json.parseToJsonElement(send(HttpMethod.Get, "$base/api/discover/3/get_web?g=all&s=$sort&p=0&gn=0&f=all&w=0")) as? JsonObject
            val items = r?.get("items").list().mapNotNull(::discoverItem)
            if (items.isEmpty()) null else SectionDto(title, items, "discover")
        }.getOrNull()
    }

    private fun discoverItem(o: JsonObject): ItemDto? {
        val h = o.obj("url_hints") ?: return null
        val root = h.str("custom_domain")?.let { "https://$it" } ?: "https://${h.str("subdomain") ?: return null}.bandcamp.com"
        val song = o.str("type") == "t"
        val url = "$root/${if (song) "track" else "album"}/${h.str("slug") ?: return null}"
        return ItemDto(if (song) "song" else "album", enc(url), o.str("primary_text") ?: return null, o.str("secondary_text"), o.long("art_id")?.let(::art), artistId = enc(root))
    }

    override suspend fun album(id: String): PageDto {
        val url = dec(id)
        val d = tralbum(url)
        val title = d.obj("current")?.str("title") ?: error("Bandcamp page has no title")
        val artist = d.str("artist")
        val root = origin(url)
        val art = d.long("art_id")?.let(::art)
        val tracks = d["trackinfo"].list().mapNotNull { t ->
            val link = t.str("title_link") ?: return@mapNotNull null
            TrackDto(enc(root + link), t.str("title") ?: return@mapNotNull null, listOfNotNull(t.str("artist") ?: artist), title, t.double("duration")?.toInt(), art, config.id, enc(root), id)
        }
        return PageDto(ItemDto("album", id, title, artist, art, artistId = enc(root)), d.obj("current")?.str("about"), tracks)
    }

    override suspend fun artist(id: String): PageDto {
        val root = dec(id)
        var html = send(HttpMethod.Get, "$root/music")
        var grid = Regex("""(?s)<li data-item-id="(album|track)-\d+".*?</li>""").findAll(html).toList()
        if (grid.isEmpty()) { html = send(HttpMethod.Get, root); grid = Regex("""(?s)<li data-item-id="(album|track)-\d+".*?</li>""").findAll(html).toList() }
        val name = Regex("""og:title" content="([^"]*)"""").find(html)?.groupValues?.get(1)?.let(::unescape)
            ?: Regex("""<title>(?:Music \| )?([^<]*)""").find(html)?.groupValues?.get(1)?.trim()?.let(::unescape) ?: "Bandcamp artist"
        val items = grid.mapNotNull { m ->
            val li = m.value
            val href = Regex("""<a href="([^"]+)"""").find(li)?.groupValues?.get(1) ?: return@mapNotNull null
            val title = Regex("""(?s)class="title">\s*([^<]*?)\s*<""").find(li)?.groupValues?.get(1)?.let(::unescape) ?: return@mapNotNull null
            val img = Regex("""<img[^>]*?(?:data-original|src)="([^"]+)"""").find(li)?.groupValues?.get(1)
            ItemDto(if (m.groupValues[1] == "track") "song" else "album", enc(URI(root + "/").resolve(href).toString()), title, name, img?.let { up(it) }, artistId = id)
        }
        return PageDto(ItemDto("artist", id, name, thumbnail = items.firstNotNullOfOrNull { it.thumbnail }), sections = if (items.isEmpty()) emptyList() else listOf(SectionDto("Discography", items)))
    }

    /** The page's `data-tralbum` JSON (with `artist` and `url` filled in). */
    private suspend fun tralbum(url: String): JsonObject {
        val raw = Regex("""data-tralbum="([^"]*)"""").find(send(HttpMethod.Get, url))?.groupValues?.get(1) ?: error("Not a Bandcamp track or album page")
        return Json.parseToJsonElement(unescape(raw)) as? JsonObject ?: error("Unreadable Bandcamp page data")
    }

    private suspend fun send(method: HttpMethod, url: String, json: String? = null): String {
        val r = http.request(url) {
            this.method = method
            if (json != null) { contentType(ContentType.Application.Json); setBody(json) }
        }
        check(r.status.isSuccess()) { "Bandcamp HTTP ${r.status.value} for ${url.substringAfter("://").substringBefore('?')}" }
        return r.bodyAsText()
    }

    companion object {
        const val ID = "bandcamp"
        const val NAME = "Bandcamp"

        private val http = HttpClient(OkHttp) {
            install(HttpTimeout) { connectTimeoutMillis = 15_000; requestTimeoutMillis = 30_000; socketTimeoutMillis = 30_000 }
            defaultRequest { header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0") }
        }

        private fun enc(url: String) = Base64.getUrlEncoder().withoutPadding().encodeToString(url.trimEnd('/').toByteArray())

        private fun dec(id: String): String {
            val u = runCatching { String(Base64.getUrlDecoder().decode(id)) }.getOrNull()
            require(u != null && (u.startsWith("https://") || u.startsWith("http://"))) { "bad Bandcamp id" }
            return u
        }

        private fun art(id: Long) = "https://f4.bcbits.com/img/a${id}_10.jpg"
        private fun up(src: String) = src.replace(Regex("""_\d+\.jpg$"""), "_10.jpg")
        private fun origin(url: String) = URI(url).let { "${it.scheme}://${it.authority}" }

        private fun unescape(s: String) = s.replace("&quot;", "\"").replace("&#39;", "'").replace("&#x27;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
    }
}

private fun JsonElement?.list() = (this as? JsonArray)?.mapNotNull { it as? JsonObject } ?: emptyList()
private fun JsonObject.obj(k: String) = this[k] as? JsonObject
private fun JsonObject.str(k: String) = (this[k] as? JsonPrimitive)?.contentOrNull
private fun JsonObject.long(k: String) = (this[k] as? JsonPrimitive)?.doubleOrNull?.toLong()
private fun JsonObject.double(k: String) = (this[k] as? JsonPrimitive)?.doubleOrNull
private fun JsonObject.bool(k: String) = (this[k] as? JsonPrimitive)?.booleanOrNull == true
