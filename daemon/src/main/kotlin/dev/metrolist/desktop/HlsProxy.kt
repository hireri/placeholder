package dev.metrolist.desktop

import com.metrolist.music.utils.DesktopPaths
import io.ktor.http.ContentType
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.server.application.ApplicationCall
import io.ktor.server.response.header
import io.ktor.server.response.respond
import io.ktor.server.response.respondBytes
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.File
import java.io.RandomAccessFile
import java.net.URI
import java.net.http.HttpClient
import java.net.http.HttpRequest
import java.net.http.HttpResponse

/**
 * Serves an fMP4 HLS stream as one byte-rangeable file, because the player reads a single URL with Range requests.
 * `init.mp4` plus the `.m4s` segments concatenated is a valid fragmented MP4 and the player's demuxer reads it as is.
 * ponytail: the whole track is fetched before the first byte is served (a few seconds for a song, long for hour-long mixes);
 * serve the prefix as it arrives if that matters.
 */
object Hls {
    /** Set once the server is listening; stream URLs handed to the client point back here. */
    @Volatile var base = ""
    @Volatile var token = ""

    private class Entry(val playlistUrl: String) {
        private val lock = Mutex()
        var file: File? = null
        suspend fun file(): File = lock.withLock { file ?: assemble(playlistUrl).also { file = it } }
    }

    /** Insertion-ordered so the oldest stream is the one evicted. */
    private val entries = java.util.Collections.synchronizedMap(LinkedHashMap<String, Entry>())
    private val http = HttpClient.newHttpClient()
    private const val KEEP = 8
    private val dir get() = File(DesktopPaths.dataDir, "cache/hls")

    init { dir.listFiles()?.forEach { it.delete() } } // leftovers from a previous run

    /** A stream the client can fetch from this daemon; `playlistUrl` is an HLS media playlist. */
    fun stream(playlistUrl: String, template: StreamDto): StreamDto {
        val key = java.util.UUID.randomUUID().toString().replace("-", "")
        entries[key] = Entry(playlistUrl)
        prune()
        return template.copy(url = "$base/hls/$key", headers = mapOf("X-Metrolist-Token" to token), mimeType = "audio/mp4")
    }

    suspend fun serve(call: ApplicationCall, key: String) {
        val entry = entries[key] ?: return call.respond(HttpStatusCode.NotFound)
        val file = entry.file()
        val total = file.length()
        val m = Regex("""bytes=(\d*)-(\d*)""").matchEntire(call.request.headers[HttpHeaders.Range] ?: "")
        val start = m?.groupValues?.get(1)?.toLongOrNull() ?: 0L
        val end = minOf(m?.groupValues?.get(2)?.toLongOrNull() ?: (total - 1), total - 1)
        if (start > end) {
            call.response.header(HttpHeaders.ContentRange, "bytes */$total")
            return call.respond(HttpStatusCode.RequestedRangeNotSatisfiable)
        }
        val bytes = withContext(Dispatchers.IO) {
            RandomAccessFile(file, "r").use { f -> ByteArray((end - start + 1).toInt()).also { f.seek(start); f.readFully(it) } }
        }
        call.response.header(HttpHeaders.AcceptRanges, "bytes")
        if (m == null) return call.respondBytes(bytes, ContentType("audio", "mp4"))
        call.response.header(HttpHeaders.ContentRange, "bytes $start-$end/$total")
        call.respondBytes(bytes, ContentType("audio", "mp4"), HttpStatusCode.PartialContent)
    }

    internal suspend fun assemble(playlistUrl: String): File = withContext(Dispatchers.IO) {
        val (init, segments) = parse(playlistUrl, fetchText(playlistUrl))
        if (segments.isEmpty()) error("HLS playlist has no segments")
        dir.mkdirs()
        val out = File.createTempFile("hls", ".mp4", dir)
        try {
            out.outputStream().buffered().use { o ->
                init?.let { o.write(fetchBytes(it)) }
                // A few segments in flight at a time, written strictly in order.
                for (batch in segments.chunked(6)) coroutineScope { batch.map { async { fetchBytes(it) } }.awaitAll() }.forEach(o::write)
            }
        } catch (e: Throwable) { out.delete(); throw e }
        out
    }

    /** Init segment (from `#EXT-X-MAP`, if any) and the segment URLs, resolved against the playlist. */
    internal fun parse(playlistUrl: String, text: String): Pair<String?, List<String>> {
        val base = URI(playlistUrl)
        val lines = text.lineSequence().map(String::trim).filter(String::isNotEmpty).toList()
        require(lines.none { it.startsWith("#EXT-X-KEY") && !it.contains("METHOD=NONE") }) { "This track is encrypted and cannot be played" }
        val init = lines.firstOrNull { it.startsWith("#EXT-X-MAP:") }?.let { Regex("""URI="([^"]+)"""").find(it)?.groupValues?.get(1) }?.let { base.resolve(it).toString() }
        return init to lines.filterNot { it.startsWith("#") }.map { base.resolve(it).toString() }
    }

    private fun fetchBytes(url: String): ByteArray = http.send(HttpRequest.newBuilder(URI(url)).header("User-Agent", "Mozilla/5.0").build(), HttpResponse.BodyHandlers.ofByteArray())
        .also { check(it.statusCode() in 200..299) { "HLS fetch failed (${it.statusCode()})" } }.body()

    private fun fetchText(url: String) = String(fetchBytes(url))

    /** Drops the oldest streams and their files so the cache stays small. */
    private fun prune() = synchronized(entries) {
        while (entries.size > KEEP) entries.remove(entries.keys.first())?.file?.delete()
    }
}
