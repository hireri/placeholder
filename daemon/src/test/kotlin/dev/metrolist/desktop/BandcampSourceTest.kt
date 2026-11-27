package dev.metrolist.desktop

import com.sun.net.httpserver.HttpServer
import kotlinx.coroutines.runBlocking
import java.net.InetSocketAddress
import java.util.Base64
import kotlin.test.AfterTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue

class BandcampSourceTest {
    private val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
    private val base get() = "http://127.0.0.1:${server.address.port}"
    private fun id(url: String) = Base64.getUrlEncoder().withoutPadding().encodeToString(url.toByteArray())

    private fun route(path: String, body: () -> String) = server.createContext(path) { ex ->
        val b = body().toByteArray()
        ex.sendResponseHeaders(200, b.size.toLong()); ex.responseBody.use { it.write(b) }
    }

    private fun tralbum(json: String) = """<div data-tralbum="${json.replace("\"", "&quot;")}"></div>"""

    private fun source(): BandcampSource {
        route("/api/bcsearch_public_api/1/autocomplete_elastic") {
            """{"auto":{"results":[
                {"type":"t","name":"Song","band_name":"Band","album_name":"LP","art_id":5,"item_url_root":"$base","item_url_path":"$base/track/song"},
                {"type":"a","name":"LP","band_name":"Band","art_id":6,"item_url_root":"$base","item_url_path":"$base/album/lp"},
                {"type":"b","name":"Band","genre_name":"Rock","art_id":7,"item_url_root":"$base"}]}}"""
        }
        route("/api/discover/3/get_web") {
            """{"items":[{"type":"a","primary_text":"New LP","secondary_text":"Band","art_id":9,"url_hints":{"subdomain":"band","custom_domain":"band.example","slug":"new-lp"}}]}"""
        }
        route("/album/lp") {
            tralbum("""{"artist":"Band","art_id":6,"current":{"title":"LP","about":"hi"},"is_preorder":false,"trackinfo":[
                {"title":"Song","title_link":"/track/song","duration":61.5,"unreleased_track":false,"file":{"mp3-128":"https://t4.bcbits.com/stream/x"}},
                {"title":"Locked","title_link":"/track/locked","duration":10,"file":null}]}""")
        }
        route("/track/song") { tralbum("""{"is_preorder":false,"trackinfo":[{"title":"Song","duration":61.5,"file":{"mp3-128":"https://t4.bcbits.com/stream/x"}}]}""") }
        route("/track/locked") { tralbum("""{"is_preorder":false,"trackinfo":[{"title":"Locked","file":null}]}""") }
        route("/track/pre") { tralbum("""{"is_preorder":true,"trackinfo":[{"title":"Pre","file":{"mp3-128":"https://t4.bcbits.com/stream/y"}}]}""") }
        route("/music") {
            """<meta property="og:title" content="Band &amp; Co"><ol><li data-item-id="album-1" class="x"><a href="/album/lp"><div class="art"><img src="https://f4.bcbits.com/img/a6_2.jpg"></div>
                <p class="title">
                  LP
                </p></a></li><li data-item-id="track-2"><a href="/track/song"><img src="https://f4.bcbits.com/img/a5_2.jpg"><p class="title">Song<br></p></a></li></ol>"""
        }
        server.start()
        return BandcampSource().also { it.base = base }
    }

    @AfterTest fun stop() = server.stop(0)

    @Test
    fun searchMapsAllThreeTypes() = runBlocking {
        val s = source()
        assertEquals(ItemDto("song", id("$base/track/song"), "Song", "Band", "https://f4.bcbits.com/img/a5_10.jpg", artistId = id(base), album = "LP"), s.search("x", "song").first { it.type == "song" })
        assertEquals("album", s.search("x", "album").first { it.type == "album" }.type)
        assertEquals(ItemDto("artist", id(base), "Band", "Rock", "https://f4.bcbits.com/img/a7_10.jpg"), s.search("x", "artist").first { it.type == "artist" })
        assertEquals(emptyList(), s.search("x", "video"))
    }

    @Test
    fun homeIsDiscoverShelves() = runBlocking {
        val sec = source().home()
        assertEquals(listOf("Popular", "New releases"), sec.map { it.title })
        assertEquals("discover", sec[0].shelf)
        assertEquals(id("https://band.example/album/new-lp"), sec[0].items.single().id)
    }

    @Test
    fun albumListsStreamableTrackIds() = runBlocking {
        val p = source().album(id("$base/album/lp"))
        assertEquals(listOf("Song", "Locked"), p.tracks.map { it.title })
        assertEquals(id("$base/track/song"), p.tracks[0].id)
        assertEquals(61, p.tracks[0].durationSec)
        assertEquals("bandcamp", p.tracks[0].source)
    }

    @Test
    fun artistPageListsReleases() = runBlocking {
        val p = source().artist(id(base))
        assertEquals("Band & Co", p.item.title)
        assertEquals(listOf("album", "song"), p.sections.single().items.map { it.type })
        assertEquals("LP", p.sections.single().items[0].title)
        assertEquals("https://f4.bcbits.com/img/a6_10.jpg", p.item.thumbnail)
    }

    @Test
    fun streamIsProgressiveMp3AndUnstreamableIsAClearError() = runBlocking {
        val s = source()
        val st = s.stream(id("$base/track/song"))
        assertEquals("https://t4.bcbits.com/stream/x", st.url)
        assertEquals("audio/mpeg", st.mimeType)
        assertTrue(st.headers.isEmpty() && !st.useRangeChunks)
        assertTrue(assertFailsWith<IllegalStateException> { s.stream(id("$base/track/locked")) }.message!!.contains("not streamable"))
        assertFailsWith<IllegalStateException> { s.stream(id("$base/track/pre")) }
        assertFailsWith<IllegalArgumentException> { s.stream("!!") }
        Unit
    }
}
