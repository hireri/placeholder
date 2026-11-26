package dev.metrolist.desktop

import com.metrolist.music.utils.potoken.PoTokenGenerator
import com.metrolist.music.utils.potoken.PoTokenResult
import com.music.bitchord.data.innertube.potoken.PO_TOKEN_HTML
import com.music.bitchord.data.innertube.potoken.parseChallengeData
import com.music.bitchord.data.innertube.potoken.parseIntegrityTokenData
import com.music.bitchord.data.innertube.potoken.stringToU8
import com.music.bitchord.data.innertube.potoken.u8ToBase64
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import timber.log.Timber
import java.io.File
import java.net.URI
import java.net.http.HttpClient
import java.net.http.HttpRequest
import java.net.http.HttpResponse
import java.time.Instant
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong
import kotlin.coroutines.cancellation.CancellationException

/**
 * BotGuard PoTokens for WEB_REMIX and friends, which have no WebView here: the `serafin-potoken` helper
 * (path in SERAFIN_POTOKEN_HELPER, set by the client) hosts the page, this does the network calls.
 * One page per visitor data, reused until its integrity token expires.
 * ponytail: no proxy support on the two botguard calls, add it if proxied users report init failures.
 */
object PoToken {
    private const val TAG = "PoToken"
    private const val API_KEY = "AIzaSyDyT5W0Jh49F30Pqqtyfdf7pDLFKLJoAnw"
    private const val REQUEST_KEY = "O43z0dpjhgX20SCx4KAo"
    private const val USER_AGENT = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) " +
        "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.3"

    private val http = HttpClient.newHttpClient()
    private val lock = Mutex()
    private var page: Page? = null

    fun install() {
        PoTokenGenerator.provider = { videoId, visitorData -> mint(videoId, visitorData) }
    }

    private suspend fun mint(videoId: String, visitorData: String): PoTokenResult? {
        val helper = System.getenv("SERAFIN_POTOKEN_HELPER")?.takeIf { File(it).isFile }
        if (helper == null) {
            Timber.tag(TAG).w("serafin-potoken helper not found; no web PoToken")
            return null
        }
        return try {
            withTimeout(60_000) { attempt(helper, videoId, visitorData, retry = true) }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            Timber.tag(TAG).w("PoToken failed: ${e.message}")
            null
        }
    }

    private suspend fun attempt(helper: String, videoId: String, visitorData: String, retry: Boolean): PoTokenResult {
        val p = lock.withLock {
            page?.takeIf { it.visitorData == visitorData && it.alive && !it.expired }
                ?: run {
                    page?.close()
                    Page.create(helper, visitorData).also { page = it }
                }
        }
        return try {
            PoTokenResult(playerRequestPoToken = p.streamingPot, streamingDataPoToken = p.mint(videoId))
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            lock.withLock { if (page === p) page = null }
            p.close()
            if (!retry) throw e
            Timber.tag(TAG).w("mint failed (${e.message}); recreating page")
            attempt(helper, videoId, visitorData, retry = false)
        }
    }

    private class Page(private val process: Process, val visitorData: String) {
        lateinit var streamingPot: String
        var expiresAt: Instant = Instant.MAX
        private val ids = AtomicLong(1)
        private val pending = ConcurrentHashMap<Long, CompletableDeferred<String>>()
        private val ready = CompletableDeferred<Unit>()
        val alive get() = process.isAlive
        val expired get() = Instant.now().isAfter(expiresAt)

        init {
            Thread({
                process.inputStream.bufferedReader().forEachLine(::onLine)
                val gone = IllegalStateException("helper exited")
                ready.completeExceptionally(gone)
                pending.values.forEach { it.completeExceptionally(gone) }
            }, "potoken-reader").apply { isDaemon = true }.start()
        }

        private fun onLine(line: String) {
            val o = runCatching { Json.parseToJsonElement(line).jsonObject }.getOrNull() ?: return
            val id = o["id"]?.jsonPrimitive?.long ?: return
            if (id == 0L) { ready.complete(Unit); return }
            val value = o["value"]?.jsonPrimitive?.content.orEmpty()
            val waiting = pending[id] ?: return
            if (o["ok"]?.jsonPrimitive?.boolean == true) waiting.complete(value)
            else waiting.completeExceptionally(IllegalStateException(value.lineSequence().first()))
        }

        fun close() = process.destroy()

        suspend fun eval(body: String): String {
            val id = ids.getAndIncrement()
            val reply = CompletableDeferred<String>().also { pending[id] = it }
            try {
                val msg = JsonObject(mapOf("id" to JsonPrimitive(id), "js" to JsonPrimitive(body)))
                withContext(Dispatchers.IO) { process.outputStream.apply { write("$msg\n".toByteArray()); flush() } }
                return withTimeout(30_000) { reply.await() }
            } finally {
                pending.remove(id)
            }
        }

        suspend fun mint(identifier: String): String =
            u8ToBase64(eval("const t = await obtainPoToken(${stringToU8(identifier)}); return t.join(',')"))

        companion object {
            suspend fun create(helper: String, visitorData: String): Page {
                Timber.tag(TAG).d("Creating PoToken page")
                val html = File.createTempFile("potoken", ".html").apply { deleteOnExit(); writeText(PO_TOKEN_HTML) }
                val process = withContext(Dispatchers.IO) {
                    ProcessBuilder(helper, html.path).redirectError(ProcessBuilder.Redirect.INHERIT).start()
                }
                val page = Page(process, visitorData)
                try {
                    withTimeout(20_000) { page.ready.await() }
                    val challenge = parseChallengeData(post("https://www.youtube.com/api/jnn/v1/Create", "[ \"$REQUEST_KEY\" ]"))
                    val botguardResponse = page.eval("const r = await runBotGuard($challenge); window.webPoSignalOutput = r.webPoSignalOutput; return r.botguardResponse")
                    val (integrityToken, seconds) = parseIntegrityTokenData(post("https://www.youtube.com/api/jnn/v1/GenerateIT", "[ \"$REQUEST_KEY\", \"$botguardResponse\" ]"))
                    page.eval("window.integrityToken = $integrityToken; await createPoTokenMinter(window.webPoSignalOutput, window.integrityToken); return 'ok'")
                    page.expiresAt = Instant.now().plusSeconds(seconds).minusSeconds(600)
                    page.streamingPot = page.mint(visitorData)
                    Timber.tag(TAG).d("Streaming PoToken generated")
                    return page
                } catch (e: Throwable) {
                    process.destroy()
                    throw e
                }
            }

            private suspend fun post(url: String, body: String): String = withContext(Dispatchers.IO) {
                val req = HttpRequest.newBuilder(URI(url))
                    .header("User-Agent", USER_AGENT)
                    .header("Accept", "application/json")
                    .header("Content-Type", "application/json+protobuf")
                    .header("x-goog-api-key", API_KEY)
                    .header("x-user-agent", "grpc-web-javascript/0.1")
                    .POST(HttpRequest.BodyPublishers.ofString(body))
                    .build()
                val res = http.send(req, HttpResponse.BodyHandlers.ofString())
                check(res.statusCode() == 200 && res.body().isNotEmpty()) { "botguard ${url.substringAfterLast('/')} failed (code=${res.statusCode()})" }
                res.body()
            }
        }
    }
}
