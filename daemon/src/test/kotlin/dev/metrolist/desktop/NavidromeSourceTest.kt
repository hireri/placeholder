package dev.metrolist.desktop

import com.sun.net.httpserver.HttpServer
import kotlinx.coroutines.runBlocking
import java.net.InetSocketAddress
import java.security.MessageDigest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class NavidromeSourceTest {
    private val seen = mutableListOf<String>()

    private fun server(): Pair<HttpServer, NavidromeSource> {
        val s = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
        s.createContext("/rest/") { ex ->
            seen += ex.requestURI.toString()
            val q = ex.requestURI.query
            val body = when (ex.requestURI.path.substringAfterLast('/')) {
                "ping.view" -> if ("u=bad" in q) """{"subsonic-response":{"status":"failed","error":{"code":40,"message":"Wrong"}}}"""
                    else """{"subsonic-response":{"status":"ok","type":"navidrome","serverVersion":"0.53.3"}}"""
                "search3.view" -> """{"subsonic-response":{"status":"ok","searchResult3":{"song":[{"id":"s1","title":"T","artist":"A","album":"Al","albumId":"a1","artistId":"r1","duration":120,"coverArt":"c1"}]}}}"""
                "getSong.view" -> """{"subsonic-response":{"status":"ok","song":{"id":"s1","contentType":"audio/flac","duration":120,"bitRate":900,"replayGain":{"trackGain":-6.5}}}}"""
                "getAlbumList2.view" -> if ("type=newest" in q) """{"subsonic-response":{"status":"ok","albumList2":{"album":[{"id":"a1","name":"Al","artist":"A","year":2020}]}}}"""
                    else """{"subsonic-response":{"status":"ok","albumList2":{}}}"""
                else -> """{"subsonic-response":{"status":"ok"}}"""
            }.toByteArray()
            ex.sendResponseHeaders(200, body.size.toLong()); ex.responseBody.use { it.write(body) }
        }
        s.start()
        return s to NavidromeSource(SourceConfig("nd", "navidrome", "ND", "http://127.0.0.1:${s.address.port}", "me", "pw"))
    }

    @Test
    fun authAndTest() = runBlocking {
        val (s, src) = server()
        val r = src.test()
        assertTrue(r.ok); assertEquals("Navidrome 0.53.3", r.detail)
        val q = seen.single().substringAfter('?').split('&').associate { it.substringBefore('=') to it.substringAfter('=') }
        val md5 = MessageDigest.getInstance("MD5").digest(("pw" + q["s"]).toByteArray()).joinToString("") { "%02x".format(it) }
        assertEquals(md5, q["t"]); assertEquals("1.16.1", q["v"]); assertFalse(seen.single().contains("pw"))
        val bad = NavidromeSource(src.config.copy(username = "bad")).test()
        assertEquals("Wrong username or password", bad.error)
        assertEquals("Can't reach the server", NavidromeSource(src.config.copy(url = "http://127.0.0.1:1")).test().error)
        s.stop(0)
    }

    @Test
    fun searchStreamHome() = runBlocking {
        val (s, src) = server()
        val song = src.search("x", "song").single()
        assertEquals("s1", song.id); assertEquals("A", song.subtitle); assertEquals("a1", song.albumId)
        assertTrue(song.thumbnail!!.contains("getCoverArt") && song.thumbnail!!.contains("t="))
        val st = src.stream("s1")
        assertEquals("audio/flac", st.mimeType); assertEquals(-11.5, st.loudnessLufs); assertTrue(st.url.contains("format=raw")); assertEquals("nd", st.client)
        val shelves = src.home()
        assertEquals(listOf("Recently added"), shelves.map { it.title }); assertEquals("recent", shelves.single().shelf)
        s.stop(0)
    }
}
