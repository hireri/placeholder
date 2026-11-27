package dev.metrolist.desktop

import com.sun.net.httpserver.HttpServer
import kotlinx.coroutines.runBlocking
import java.net.InetSocketAddress
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class HlsProxyTest {
    @Test
    fun parseResolvesInitAndSegmentsAgainstThePlaylist() {
        val (init, segs) = Hls.parse("https://cdn.example/a/b/pl.m3u8", "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:10,\nseg0.m4s\n#EXTINF:10,\n../seg1.m4s\n#EXT-X-ENDLIST\n")
        assertEquals("https://cdn.example/a/b/init.mp4", init)
        assertEquals(listOf("https://cdn.example/a/b/seg0.m4s", "https://cdn.example/a/seg1.m4s"), segs)
    }

    @Test
    fun encryptedPlaylistsAreRefused() {
        assertFailsWith<IllegalArgumentException> { Hls.parse("https://x/pl.m3u8", "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\nseg.m4s") }
    }

    @Test
    fun assembleConcatenatesInitAndSegmentsInOrder() = runBlocking {
        val parts = (0..9).associate { "/s$it.m4s" to ByteArray(300) { b -> (it * 7 + b).toByte() } } + ("/init.mp4" to ByteArray(50) { 1 })
        val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
        val playlist = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n" + (0..9).joinToString("") { "#EXTINF:1,\ns$it.m4s\n" }
        server.createContext("/") { ex ->
            val body = if (ex.requestURI.path == "/pl.m3u8") playlist.toByteArray() else parts[ex.requestURI.path] ?: ByteArray(0)
            ex.sendResponseHeaders(if (body.isEmpty()) 404 else 200, body.size.toLong()); ex.responseBody.use { it.write(body) }
        }
        server.start()
        try {
            val f = Hls.assemble("http://127.0.0.1:${server.address.port}/pl.m3u8")
            assertContentEquals((0..9).fold(parts["/init.mp4"]!!) { acc, i -> acc + parts["/s$i.m4s"]!! }, f.readBytes())
            f.delete()
        } finally { server.stop(0) }
        Unit
    }
}
