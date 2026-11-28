package dev.metrolist.desktop

import com.sun.net.httpserver.HttpExchange
import com.sun.net.httpserver.HttpServer
import kotlinx.coroutines.runBlocking
import java.net.InetSocketAddress
import java.util.Collections
import kotlin.test.AfterTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class JellyfinSourceTest {
    private val seen = Collections.synchronizedList(mutableListOf<String>())
    private var logins = 0
    private var rejectNext = false
    private var password = "pw"
    private val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0).apply {
        createContext("/") { x ->
            seen += "${x.requestMethod} ${x.requestURI}"
            val path = x.requestURI.path
            val body = x.requestBody.readBytes().decodeToString()
            when {
                path == "/System/Info/Public" -> x.reply(200, """{"ProductName":"Jellyfin Server","Version":"10.9.11"}""")
                path == "/Users/AuthenticateByName" -> if (body.contains("\"Pw\":\"$password\"")) {
                    logins++
                    x.reply(200, """{"AccessToken":"tok$logins","User":{"Id":"u1"}}""")
                } else x.reply(401, "")
                rejectNext && x.requestHeaders.getFirst("X-Emby-Token") == "tok1" -> { rejectNext = false; x.reply(401, "") }
                path == "/Items" && x.requestURI.query.contains("Audio") -> x.reply(200, ITEMS)
                path == "/Items" && x.requestURI.query.contains("Ids=") -> x.reply(200, """{"Items":[$SONG2,$SONG]}""")
                path == "/Items/s1" -> x.reply(200, SONG)
                path == "/UserItems/Resume" || path == "/Items/Latest" -> x.reply(200, if (path == "/Items/Latest") "[]" else ITEMS)
                path == "/Items/s1/InstantMix" -> x.reply(200, ITEMS)
                path.startsWith("/Sessions/") || path.startsWith("/UserFavoriteItems/") -> x.reply(204, "")
                else -> x.reply(200, """{"Items":[]}""")
            }
        }
        start()
    }
    private fun HttpExchange.reply(code: Int, s: String) {
        val b = s.toByteArray()
        sendResponseHeaders(code, if (code == 204) -1 else b.size.toLong())
        if (code != 204) responseBody.use { it.write(b) }
        close()
    }
    private fun source() = JellyfinSource(SourceConfig("jellyfin-t", "jellyfin", "J", "http://127.0.0.1:${server.address.port}/", "aveline", "pw"))

    @AfterTest fun stop() = server.stop(0)

    @Test fun testReportsVersionAndWrongPassword() = runBlocking {
        val ok = source().test()
        assertTrue(ok.ok); assertEquals("Jellyfin 10.9.11 as aveline", ok.detail)
        password = "other"
        val bad = source().test()
        assertFalse(bad.ok); assertEquals("Wrong username or password", bad.error)
    }

    @Test fun reauthenticatesOnceOn401() = runBlocking {
        val s = source()
        s.search("x", "song")
        rejectNext = true
        s.search("x", "song")
        assertEquals(2, logins)
    }

    @Test fun searchMapsSongs() = runBlocking {
        val r = source().search("hey", "song").single()
        assertEquals("song", r.type); assertEquals("s1", r.id); assertEquals("Artist", r.subtitle)
        assertEquals(215, r.durationSec); assertEquals("a1", r.albumId); assertEquals("ar1", r.artistId)
        assertTrue(r.thumbnail!!.contains("/Items/s1/Images/Primary") && r.thumbnail!!.contains("api_key=tok1"))
        assertTrue(seen.any { it.contains("searchTerm=hey") && it.contains("IncludeItemTypes=Audio") })
    }

    @Test fun streamIsProgressiveOriginal() = runBlocking {
        val s = source().stream("s1")
        assertTrue(s.url.endsWith("/Audio/s1/stream?static=true&api_key=tok1"))
        assertEquals("audio/flac", s.mimeType); assertEquals(215, s.durationSec); assertEquals(900000, s.bitrate)
        assertEquals(-10.0, s.loudnessLufs); assertTrue(s.headers.isEmpty()); assertFalse(s.useRangeChunks)
    }

    @Test fun homeSkipsEmptyShelvesAndReportsPlay() = runBlocking {
        val src = source()
        val shelves = src.home().mapNotNull { it.shelf }
        assertEquals(listOf("continue", "played", "mixes"), shelves)
        src.reportPlay("s1", "stop", 0.0)
        assertTrue(seen.any { it.startsWith("POST /Sessions/Playing/Stopped") })
    }

    @Test fun tracksByIdsKeepsRequestedOrderAndSkipsUnknown() = runBlocking {
        val r = source().tracksByIds(listOf("jellyfin-t:s1", "jellyfin-t:nope", "jellyfin-t:s2"))
        assertEquals(listOf("s1", "s2"), r.map { it.id })
        assertEquals(1, seen.count { it.startsWith("GET /Items?") && it.contains("Ids=s1%2Cnope%2Cs2") })
    }

    private companion object {
        const val SONG2 = """{"Id":"s2","Name":"Two","Type":"Audio","Artists":["Artist"]}"""
        const val SONG = """{"Id":"s1","Name":"Hey","Type":"Audio","Artists":["Artist"],"ArtistItems":[{"Id":"ar1","Name":"Artist"}],"Album":"Alb","AlbumId":"a1","RunTimeTicks":2150000000,"ImageTags":{"Primary":"x"},"NormalizationGain":-8.0,"MediaSources":[{"Container":"flac","Bitrate":900000}]}"""
        const val ITEMS = """{"Items":[$SONG]}"""
    }
}
