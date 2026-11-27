package dev.metrolist.desktop

import com.sun.net.httpserver.HttpExchange
import com.sun.net.httpserver.HttpServer
import kotlinx.coroutines.runBlocking
import java.net.InetSocketAddress
import java.util.Collections
import kotlin.test.AfterTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class NavidromeWriteTest {
    private val seen = Collections.synchronizedList(mutableListOf<String>())
    private var imageType = ""
    private val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0).apply {
        createContext("/") { x ->
            val body = x.requestBody.readBytes()
            seen += "${x.requestMethod} ${x.requestURI}"
            val path = x.requestURI.path
            when {
                path == "/auth/login" -> x.reply("""{"token":"jwt1"}""")
                path.endsWith("/image") -> { imageType = x.requestHeaders.getFirst("Content-Type").orEmpty(); seen += "auth=" + x.requestHeaders.getFirst("X-ND-Authorization") + " bytes=" + body.size; x.reply("{}") }
                path.endsWith("createPlaylist.view") -> x.reply("""{"subsonic-response":{"status":"ok","playlist":{"id":"p7"}}}""")
                path.endsWith("getPlaylist.view") -> x.reply("""{"subsonic-response":{"status":"ok","playlist":{"id":"p1","name":"P","entry":[{"id":"a"},{"id":"b"},{"id":"a"},{"id":"c"}]}}}""")
                else -> x.reply("""{"subsonic-response":{"status":"ok"}}""")
            }
        }
        start()
    }

    private fun HttpExchange.reply(s: String) {
        val b = s.toByteArray()
        sendResponseHeaders(200, b.size.toLong()); responseBody.use { it.write(b) }
    }

    private fun source() = NavidromeSource(SourceConfig("nd-w", "navidrome", "ND", "http://127.0.0.1:${server.address.port}", "me", "pw"))

    @AfterTest fun stop() = server.stop(0)

    @Test fun playlistWritesUseSubsonicCalls() = runBlocking {
        val s = source()
        assertEquals("p7", s.createPlaylist("Mix", listOf("a", "b")))
        assertTrue(seen.any { "createPlaylist.view" in it && "name=Mix" in it && "songId=a" in it && "songId=b" in it })
        s.renamePlaylist("p1", "New")
        assertTrue(seen.any { "updatePlaylist.view" in it && "name=New" in it && "playlistId=p1" in it })
        s.addToPlaylist("p1", listOf("x", "y"))
        assertTrue(seen.any { "songIdToAdd=x" in it && "songIdToAdd=y" in it })
        s.removeFromPlaylist("p1", listOf("a"))
        // Subsonic removes by position: song "a" sits at 0 and 2.
        assertTrue(seen.any { "songIndexToRemove=0" in it && "songIndexToRemove=2" in it && "songIndexToRemove=1" !in it })
        s.deletePlaylist("p1")
        assertTrue(seen.any { "deletePlaylist.view" in it && "id=p1" in it })
        Unit
    }

    @Test fun playlistImageLogsInThenPostsAForm() = runBlocking {
        source().setImage("playlist", "p1", ByteArray(40) { 7 }, "image/jpeg")
        assertTrue(seen.contains("POST /auth/login"))
        assertTrue(seen.any { it == "POST /api/playlist/p1/image" })
        assertTrue(seen.any { it.startsWith("auth=Bearer jwt1") })
        assertTrue(imageType.startsWith("multipart/form-data"))
    }

    @Test fun onlyPlaylistImagesAreSupported() = runBlocking {
        val e = runCatching { source().setImage("album", "a1", ByteArray(3), "image/png") }.exceptionOrNull()
        assertTrue(e?.message.orEmpty().contains("playlist"))
        Unit
    }
}
