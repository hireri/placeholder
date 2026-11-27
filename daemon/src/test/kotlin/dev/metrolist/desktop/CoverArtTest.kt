package dev.metrolist.desktop

import com.sun.net.httpserver.HttpServer
import kotlinx.coroutines.runBlocking
import java.net.InetSocketAddress
import java.util.concurrent.atomic.AtomicInteger
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class CoverArtTest {
    @Test
    fun findsCoverMissesAndCaches() = runBlocking {
        val mbCalls = AtomicInteger()
        val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
        server.createContext("/") { ex ->
            val p = ex.requestURI.path
            val q = ex.requestURI.query.orEmpty()
            val (code, body) = when {
                p.startsWith("/ws/2/release/") -> { mbCalls.incrementAndGet(); 200 to if ("Nope" in q) """{"releases":[]}""" else if ("NoArt" in q) """{"releases":[{"id":"bad"}]}""" else """{"releases":[{"id":"abc"}]}""" }
                p.startsWith("/ws/2/recording/") -> { mbCalls.incrementAndGet(); 200 to """{"recordings":[{"releases":[{"id":"abc"}]}]}""" }
                p == "/release/abc/front-500" -> 200 to ""
                p == "/search" -> 200 to if ("Fallback" in q) """{"results":[{"artistName":"Other","artworkUrl100":"https://x/a/100x100bb.jpg"},{"artistName":"The Artist","artworkUrl100":"https://x/a/100x100bb.jpg"}]}""" else """{"results":[]}"""
                else -> 404 to ""
            }
            ex.sendResponseHeaders(code, if (body.isEmpty()) -1 else body.length.toLong())
            if (body.isNotEmpty()) ex.responseBody.use { it.write(body.toByteArray()) } else ex.close()
        }
        server.start()
        val base = "http://127.0.0.1:${server.address.port}"
        CoverArt.mbBase = base; CoverArt.caaBase = base; CoverArt.itunesBase = base; CoverArt.clear()
        try {
            assertEquals("$base/release/abc/front-500", CoverArt.lookup("Artist", "Album", "T").url)
            assertEquals("$base/release/abc/front-500", CoverArt.lookup("ARTIST", "album", "t").url)
            assertEquals(1, mbCalls.get())
            assertNull(CoverArt.lookup("Artist", "Nope", "T").url)
            assertNull(CoverArt.lookup("Artist", "NoArt", "T").url)
            val n = mbCalls.get()
            assertNull(CoverArt.lookup("Artist", "Nope", "T").url)
            assertEquals(n, mbCalls.get())
            assertEquals("$base/release/abc/front-500", CoverArt.lookup("Artist", "", "Song").url)
            // MusicBrainz has no release: the catalogue search answers, for the right artist only, at a larger size.
            assertEquals("https://x/a/600x600bb.jpg", CoverArt.lookup("The Artist", "Nope Fallback", "T").url)
        } finally { server.stop(0); CoverArt.clear() }
        Unit
    }
}
