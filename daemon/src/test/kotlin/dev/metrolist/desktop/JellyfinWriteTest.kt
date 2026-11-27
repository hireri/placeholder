package dev.metrolist.desktop

import com.sun.net.httpserver.HttpExchange
import com.sun.net.httpserver.HttpServer
import kotlinx.coroutines.runBlocking
import java.net.InetSocketAddress
import java.util.Collections
import kotlin.test.AfterTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue

class JellyfinWriteTest {
    private val seen = Collections.synchronizedList(mutableListOf<String>())
    private val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0).apply {
        createContext("/") { x ->
            val body = x.requestBody.readBytes().decodeToString()
            seen += "${x.requestMethod} ${x.requestURI} $body"
            val path = x.requestURI.path
            when {
                path == "/Users/AuthenticateByName" -> x.reply(200, """{"AccessToken":"tok","User":{"Id":"u1"}}""")
                path == "/Playlists" && x.requestMethod == "POST" -> x.reply(200, """{"Id":"p9"}""")
                path == "/Playlists/p1/Items" && x.requestMethod == "GET" -> x.reply(200, """{"Items":[{"Id":"s1","PlaylistItemId":"e1"},{"Id":"s3","PlaylistItemId":"e3"},{"Id":"s1","PlaylistItemId":"e4"}]}""")
                path == "/Items/locked" -> x.reply(403, "")
                else -> x.reply(204, "")
            }
        }
        start()
    }

    private fun HttpExchange.reply(code: Int, s: String) {
        val b = s.toByteArray()
        sendResponseHeaders(code, if (b.isEmpty()) -1 else b.size.toLong())
        if (b.isNotEmpty()) responseBody.use { it.write(b) }
        close()
    }

    private fun source() = JellyfinSource(SourceConfig("jellyfin-w", "jellyfin", "J", "http://127.0.0.1:${server.address.port}", "me", "pw"))

    @AfterTest fun stop() = server.stop(0)

    @Test fun playlistWritesUseJellyfinEndpoints() = runBlocking {
        val s = source()
        assertEquals("p9", s.createPlaylist("Mix", listOf("s1", "s2")))
        assertTrue(seen.any { it.startsWith("POST /Playlists ") && """"Name":"Mix"""" in it && """"s2"""" in it && """"UserId":"u1"""" in it })
        s.renamePlaylist("p1", "New")
        assertTrue(seen.any { it.startsWith("POST /Playlists/p1 ") && """"Name":"New"""" in it })
        s.addToPlaylist("p1", listOf("s1", "s2"))
        assertTrue(seen.any { it.startsWith("POST /Playlists/p1/Items?") && "ids=s1%2Cs2" in it })
        s.removeFromPlaylist("p1", listOf("s1"))
        // Both entries of the song that is in the playlist twice go, and the other song stays.
        assertTrue(seen.any { it.startsWith("DELETE /Playlists/p1/Items?") && "entryIds=e1%2Ce4" in it })
        s.deletePlaylist("p1")
        assertTrue(seen.any { it.startsWith("DELETE /Items/p1") })
    }

    @Test fun imageIsUploadedAsBase64WithItsMimeType() = runBlocking {
        source().setImage("playlist", "p1", byteArrayOf(1, 2, 3), "image/png")
        assertTrue(seen.any { it.startsWith("POST /Items/p1/Images/Primary ") && it.endsWith("AQID") })
    }

    @Test fun permissionRefusalIsExplained() = runBlocking {
        val e = assertFailsWith<Exception> { source().deletePlaylist("locked") }
        assertTrue("isn't allowed" in e.message.orEmpty())
        Unit
    }
}
