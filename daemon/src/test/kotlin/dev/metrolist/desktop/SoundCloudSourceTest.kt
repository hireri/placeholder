package dev.metrolist.desktop

import com.sun.net.httpserver.HttpExchange
import com.sun.net.httpserver.HttpServer
import kotlinx.coroutines.runBlocking
import java.net.InetSocketAddress
import kotlin.test.AfterTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue

class SoundCloudSourceTest {
    private val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
    private val base get() = "http://127.0.0.1:${server.address.port}"
    private var validId = "idoneaaaaaaaaaaaaaaa"
    private var webHits = 0
    private val queries = mutableListOf<String>()

    private fun route(path: String, body: (HttpExchange) -> String) = server.createContext(path) { ex ->
        val b = body(ex).toByteArray()
        ex.sendResponseHeaders(200, b.size.toLong()); ex.responseBody.use { it.write(b) }
    }

    private fun source(secret: String? = null): SoundCloudSource {
        route("/") { webHits++; """<script src="$base/assets/app.js"></script>""" }
        route("/assets/app.js") { """var x={client_id:"$validId"}""" }
        // API routes answer 401 unless the current client_id is presented.
        fun api(path: String, body: String) = server.createContext("/api$path") { ex ->
            queries += ex.requestURI.query
            val ok = ex.requestURI.query.contains("client_id=$validId")
            val b = (if (ok) body else "{}").toByteArray()
            ex.sendResponseHeaders(if (ok) 200 else 401, b.size.toLong()); ex.responseBody.use { it.write(b) }
        }
        api("/search/tracks", """{"collection":[{"kind":"track","id":42,"title":"Song","duration":123456,"artwork_url":"https://i1.sndcdn.com/a-large.jpg","user":{"id":7,"username":"Up","avatar_url":"https://i1.sndcdn.com/u-large.jpg"}},
            {"kind":"track","id":43,"title":"NoArt","duration":1000,"artwork_url":null,"user":{"id":7,"username":"Up","avatar_url":"https://i1.sndcdn.com/u-large.jpg"}}]}""")
        api("/search/albums", """{"collection":[{"kind":"playlist","id":5,"title":"LP","is_album":true,"user":{"id":7,"username":"Up"}}]}""")
        api("/search/users", """{"collection":[{"kind":"user","id":7,"username":"Up","avatar_url":"https://i1.sndcdn.com/u-large.jpg"}]}""")
        api("/playlists/5", """{"kind":"playlist","id":5,"title":"LP","is_album":true,"user":{"id":7,"username":"Up"},"tracks":[{"kind":"track","id":42,"title":"Song","user":{"id":7,"username":"Up"}},{"id":99}]}""")
        api("/tracks", """[{"kind":"track","id":99,"title":"Stub filled","user":{"id":7,"username":"Up"}}]""")
        api("/tracks/42", """{"id":42,"duration":5000,"track_authorization":"auth1","media":{"transcodings":[
            {"url":"$base/api/hls","format":{"protocol":"hls","mime_type":"audio/mpegurl"}},
            {"url":"$base/api/prog","format":{"protocol":"progressive","mime_type":"audio/mpeg"}}]}}""")
        api("/tracks/43", """{"id":43,"media":{"transcodings":[{"url":"$base/api/hls","format":{"protocol":"hls","mime_type":"audio/mpegurl"}}]}}""")
        api("/prog", """{"url":"https://cf-media.sndcdn.com/real.mp3"}""")
        api("/tracks/44", """{"id":44,"duration":9000,"track_authorization":"a","media":{"transcodings":[
            {"url":"$base/api/h96","preset":"aac_96k","format":{"protocol":"hls","mime_type":"audio/mp4; codecs=\"mp4a.40.2\""}},
            {"url":"$base/api/h160","preset":"aac_160k","format":{"protocol":"hls","mime_type":"audio/mp4; codecs=\"mp4a.40.2\""}},
            {"url":"$base/api/henc","preset":"aac_160k","format":{"protocol":"ctr-encrypted-hls","mime_type":"audio/mp4"}}]}}""")
        api("/h160", """{"url":"https://cf-hls-media.sndcdn.com/pl/160.m3u8"}""")
        api("/users/7", """{"kind":"user","id":7,"username":"Up","description":"bio"}""")
        api("/users/7/toptracks", """{"collection":[{"kind":"track","id":42,"title":"Song","user":{"id":7,"username":"Up"}}]}""")
        api("/users/7/albums", """{"collection":[{"kind":"playlist","id":5,"title":"LP","is_album":true,"user":{"id":7,"username":"Up"}}]}""")
        api("/users/7/playlists_without_albums", """{"collection":[]}""")
        api("/users/7/tracks", """{"collection":[{"kind":"track","id":42,"title":"Song","user":{"id":7,"username":"Up"}},
            {"kind":"track","id":50,"title":"Loose","created_at":"2020-05-01T00:00:00Z","user":{"id":7,"username":"Up"}}],"next_href":"$base/api/more?limit=50"}""")
        api("/more", """{"collection":[{"kind":"track","id":51,"title":"Later","created_at":"2022-01-01T00:00:00Z","user":{"id":7,"username":"Up"}}]}""")
        api("/me", """{"id":1,"username":"me"}""")
        server.start()
        return SoundCloudSource(SourceConfig("sc", "soundcloud", "SC", secret = secret)).also { it.api = "$base/api"; it.web = base }
    }

    @AfterTest fun stop() = server.stop(0)

    @Test
    fun mapsSearchResults() = runBlocking {
        val s = source()
        val songs = s.search("x", "song")
        assertEquals(ItemDto("song", "42", "Song", "Up", "https://i1.sndcdn.com/a-t300x300.jpg", 123, artistId = "7"), songs[0])
        assertEquals("https://i1.sndcdn.com/u-t300x300.jpg", songs[1].thumbnail)
        assertEquals("album", s.search("x", "album").single().type)
        assertEquals("artist", s.search("x", "artist").single().type)
    }

    @Test
    fun rediscoversClientIdOnceOn401() = runBlocking {
        val s = source()
        s.search("x", "song")
        validId = "idtwobbbbbbbbbbbbbbb"
        assertEquals(2, s.search("x", "song").size)
        assertEquals(2, webHits)
    }

    @Test
    fun hlsOnlyTrackGoesThroughTheProxyAtTheBestBitrate() = runBlocking {
        val st = source().stream("44")
        assertTrue("/hls/" in st.url)
        assertEquals("audio/mp4", st.mimeType)
        assertEquals(160_000, st.bitrate)
        assertTrue("X-Metrolist-Token" in st.headers)
    }

    @Test
    fun streamPrefersProgressiveAndRejectsHlsOnly() = runBlocking {
        val s = source()
        val st = s.stream("42")
        assertEquals("https://cf-media.sndcdn.com/real.mp3", st.url)
        assertEquals("audio/mpeg", st.mimeType)
        assertTrue(queries.any { it.contains("track_authorization=auth1") })
        assertFailsWith<IllegalStateException> { s.stream("43") }
        Unit
    }

    @Test
    fun playlistFillsStubTracks() = runBlocking {
        val p = source().album("5")
        assertEquals(listOf("Song", "Stub filled"), p.tracks.map { it.title })
        assertEquals("album", p.item.type)
    }

    @Test
    fun artistPageIsShelvesNotATrackList() = runBlocking {
        val p = source().artist("7")
        assertTrue(p.tracks.isEmpty())
        assertEquals(listOf("Top songs", "Albums"), p.sections.map { it.title })
        assertEquals("bio", p.description)
    }

    @Test
    fun discographyHasAlbumsAndEveryLooseTrackAcrossPages() = runBlocking {
        val d = source().discography("7")
        assertEquals(listOf("Later", "Loose", "LP"), d.map { it.item.title })
        assertEquals("t-51", d[0].item.id)
        assertEquals(listOf("Later"), d[0].tracks.map { it.title })
    }

    @Test
    fun capsAndSignInDependOnToken() = runBlocking {
        val anon = source()
        assertTrue(Cap.LIKE !in anon.caps && Cap.LIBRARY !in anon.caps)
        assertEquals("Not signed in (search and streaming only)", anon.test().detail)
        assertEquals(emptyList(), anon.library("songs"))
        assertFailsWith<UnsupportedOperationException> { anon.like("42", true) }
        val auth = SoundCloudSource(SourceConfig("sc", "soundcloud", "SC", secret = "tok")).also { it.api = "$base/api"; it.web = base }
        assertTrue(Cap.LIKE in auth.caps && Cap.LIBRARY in auth.caps)
        assertEquals("Signed in as me", auth.test().detail)
    }
}
