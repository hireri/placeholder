package dev.metrolist.desktop

import dev.metrolist.desktop.JellyfinSource.Companion.int
import dev.metrolist.desktop.JellyfinSource.Companion.long
import dev.metrolist.desktop.JellyfinSource.Companion.str
import io.ktor.http.HttpMethod
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.future.await
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonObjectBuilder
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.net.URI
import java.net.http.HttpClient
import java.net.http.WebSocket
import java.net.http.WebSocketHandshakeException
import java.time.Instant
import java.util.concurrent.CompletionStage
import java.util.concurrent.CancellationException

@Serializable data class SyncGroupDto(val id: String, val name: String, val state: String, val participants: List<String>)
@Serializable data class SyncCreatedDto(val id: String)
@Serializable data class SyncCmdReq(
    val type: String, val positionMs: Long = 0, val ids: List<String> = emptyList(),
    val index: Int = 0, val playing: Boolean = true, val itemId: String? = null,
)

/**
 * One Jellyfin source's SyncPlay session: REST commands out, a lazily opened `/socket` in, and a bounded event log the client long-polls.
 * `call` and the token functions belong to the owning [JellyfinSource] so login and 401 handling stay in one place.
 */
class SyncPlay(
    private val src: String,
    private val base: String,
    private val deviceId: String,
    private val token: suspend () -> String,
    private val refreshToken: suspend (stale: String) -> String,
    private val call: suspend (HttpMethod, String, JsonObject?) -> JsonElement?,
) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val seqFlow = MutableStateFlow(0L)
    private val buf = ArrayDeque<JsonObject>()
    @Volatile private var offsetMs = 0L
    @Volatile private var groupId: String? = null
    private var participants = linkedSetOf<String>()
    private var state = "Idle"
    @Volatile private var connection: Job? = null
    @Volatile private var ws: WebSocket? = null
    private val connected = MutableStateFlow(false)

    suspend fun groups(): List<SyncGroupDto> = (call(HttpMethod.Get, "/SyncPlay/List", null) as? JsonArray).orEmpty().mapNotNull {
        val o = it as? JsonObject ?: return@mapNotNull null
        SyncGroupDto(o.str("GroupId") ?: return@mapNotNull null, o.str("GroupName").orEmpty(), o.str("State") ?: "Idle", o.strings("Participants"))
    }

    suspend fun create(name: String): String {
        synchronized(buf) { buf.clear() }
        open()
        val r = call(HttpMethod.Post, "/SyncPlay/New", buildJsonObject { put("GroupName", name) }) as? JsonObject
        return r?.str("GroupId") ?: awaitGroup() ?: error("Jellyfin didn't confirm the SyncPlay group")
    }

    suspend fun join(id: String) {
        synchronized(buf) { buf.clear() }
        open()
        call(HttpMethod.Post, "/SyncPlay/Join", buildJsonObject { put("GroupId", id) })
    }

    suspend fun leave() {
        try { call(HttpMethod.Post, "/SyncPlay/Leave", null) } finally { close() }
    }

    suspend fun command(c: SyncCmdReq) {
        val now = Instant.ofEpochMilli(System.currentTimeMillis() + offsetMs).toString()
        val ticks = msToTicks(c.positionMs)
        val ready = { buildJsonObject {
            put("When", now); put("PositionTicks", ticks); put("IsPlaying", c.playing); c.itemId?.let { put("PlaylistItemId", it) }
        } }
        val item = { buildJsonObject { c.itemId?.let { put("PlaylistItemId", it) } } }
        when (c.type) {
            "pause" -> call(HttpMethod.Post, "/SyncPlay/Pause", null)
            "unpause" -> call(HttpMethod.Post, "/SyncPlay/Unpause", null)
            "seek" -> call(HttpMethod.Post, "/SyncPlay/Seek", buildJsonObject { put("PositionTicks", ticks) })
            "queue" -> call(HttpMethod.Post, "/SyncPlay/SetNewQueue", buildJsonObject {
                put("PlayingQueue", JsonArray(c.ids.map { JsonPrimitive(it.removePrefix("$src:")) }))
                put("PlayingItemPosition", c.index); put("StartPositionTicks", ticks)
            })
            "next" -> call(HttpMethod.Post, "/SyncPlay/NextItem", item())
            "prev" -> call(HttpMethod.Post, "/SyncPlay/PreviousItem", item())
            "ready" -> call(HttpMethod.Post, "/SyncPlay/Ready", ready())
            "buffering" -> call(HttpMethod.Post, "/SyncPlay/Buffering", ready())
            else -> throw IllegalArgumentException("Unknown SyncPlay command")
        }
    }

    suspend fun events(after: Long, waitSec: Long): JsonObject {
        val from = if (after > seqFlow.value) 0 else after // a daemon restart resets seq
        withTimeoutOrNull(waitSec.coerceIn(0, 60) * 1000) { seqFlow.first { it > from } }
        val evs = synchronized(buf) { buf.filter { it.long("seq")!! > from } }
        return buildJsonObject { put("seq", seqFlow.value); put("events", JsonArray(evs)) }
    }

    private suspend fun awaitGroup() = withTimeoutOrNull(5_000) { while (groupId == null) delay(50); groupId }

    private fun emit(type: String, fields: JsonObjectBuilder.() -> Unit) = synchronized(buf) {
        val seq = seqFlow.value + 1
        buf.addLast(buildJsonObject { put("seq", seq); put("type", type); fields() })
        while (buf.size > RING) buf.removeFirst()
        seqFlow.value = seq
    }

    private suspend fun open() {
        if (connection?.isActive != true) {
            connected.value = false
            connection = scope.launch { run() }
        }
        withTimeoutOrNull(10_000) { connected.first { it } } ?: error("Couldn't open the SyncPlay connection")
    }

    private fun close() {
        connection?.cancel(); connection = null
        ws?.abort(); ws = null
        connected.value = false
        synchronized(buf) { groupId = null; participants = linkedSetOf(); state = "Idle" }
    }

    /** Best-effort teardown when the owning source is dropped: leave the group if in one, stop the socket and scope. Never throws. */
    fun shutdown() {
        if (groupId != null) CoroutineScope(Dispatchers.IO).launch { runCatching { call(HttpMethod.Post, "/SyncPlay/Leave", null) } }
        runCatching { close() }
        scope.cancel()
    }

    private suspend fun run() {
        var backoff = 1_000L
        var tok = token()
        while (scope.isActive) {
            val closed = CompletableDeferred<Unit>()
            try {
                syncClock()
                val url = base.replaceFirst("http", "ws") + "/socket?api_key=$tok&deviceId=$deviceId"
                val sb = StringBuilder()
                val socket = client.newWebSocketBuilder().buildAsync(URI(url), object : WebSocket.Listener {
                    override fun onText(w: WebSocket, data: CharSequence, last: Boolean): CompletionStage<*>? {
                        sb.append(data)
                        if (last) { val m = sb.toString(); sb.setLength(0); onMessage(w, m) }
                        w.request(1)
                        return null
                    }
                    override fun onClose(w: WebSocket, code: Int, reason: String): CompletionStage<*>? { closed.complete(Unit); return null }
                    override fun onError(w: WebSocket, e: Throwable) { closed.complete(Unit) }
                }).await()
                ws = socket
                connected.value = true
                backoff = 1_000
                var idle = 0
                while (!closed.isCompleted) {
                    delay(KEEPALIVE_MS)
                    socket.sendText("""{"MessageType":"KeepAlive"}""", true)
                    // ponytail: idle timer counts keepalive ticks; a group that never forms within ~2 min drops the socket
                    idle = if (groupId == null) idle + 1 else 0
                    if (idle >= 4) { close(); return }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                val hs = generateSequence(e as Throwable) { it.cause }.filterIsInstance<WebSocketHandshakeException>().firstOrNull()
                if (hs?.response?.statusCode() == 401) tok = runCatching { refreshToken(tok) }.getOrDefault(tok)
            }
            connected.value = false
            if (groupId != null) emit("left") {}
            groupId = null
            delay(backoff)
            backoff = (backoff * 2).coerceAtMost(30_000)
        }
    }

    private suspend fun syncClock() {
        val t0 = System.currentTimeMillis()
        val o = call(HttpMethod.Get, "/GetUtcTime", null) as? JsonObject ?: return
        val t3 = System.currentTimeMillis()
        val t1 = parseInstantMs(o.str("RequestReceptionTime")) ?: return
        val t2 = parseInstantMs(o.str("ResponseTransmissionTime")) ?: return
        offsetMs = clockOffset(t0, t1, t2, t3)
    }

    private fun onMessage(w: WebSocket, text: String) {
        val m = runCatching { json.parseToJsonElement(text) as JsonObject }.getOrNull() ?: return
        val data = m["Data"]
        when (m.str("MessageType")) {
            "ForceKeepAlive" -> w.sendText("""{"MessageType":"KeepAlive"}""", true)
            "SyncPlayCommand" -> (data as? JsonObject)?.let { commandEvent(it, offsetMs) }?.let { f -> emit("command", f) }
            "SyncPlayGroupUpdate" -> (data as? JsonObject)?.let { groupUpdate(it.str("Type"), it["Data"]) }
        }
    }

    private fun groupUpdate(type: String?, d: JsonElement?) = synchronized(buf) {
        when (type) {
            "GroupJoined" -> (d as? JsonObject)?.let {
                groupId = it.str("GroupId"); participants = LinkedHashSet(it.strings("Participants")); state = it.str("State") ?: "Idle"
                emitGroup()
            }
            "UserJoined" -> d.str()?.let { participants += it; emitGroup() }
            "UserLeft" -> d.str()?.let { participants -= it; emitGroup() }
            "StateUpdate" -> (d as? JsonObject)?.str("State")?.let { state = it; emitGroup() }
            "PlayQueue" -> (d as? JsonObject)?.let { emit("queue", queueEvent(it, src)) }
            "GroupLeft", "NotInGroup", "GroupDoesNotExist", "LibraryAccessDenied" -> { groupId = null; emit("left") {} }
        }
    }

    private fun emitGroup() { val g = groupId ?: return; emit("group") { put("groupId", g); put("participants", JsonArray(participants.map(::JsonPrimitive))); put("state", state) } }

    companion object {
        private const val RING = 200
        private const val KEEPALIVE_MS = 30_000L
        private val json = Json { ignoreUnknownKeys = true }
        private val client = HttpClient.newHttpClient()

        fun msToTicks(ms: Long) = ms * 10_000
        fun ticksToMs(t: Long) = t / 10_000
        /** Server clock minus local clock, from one NTP-style round trip. */
        fun clockOffset(t0: Long, t1: Long, t2: Long, t3: Long) = ((t1 - t0) + (t2 - t3)) / 2
        fun parseInstantMs(s: String?) = s?.let { runCatching { Instant.parse(it).toEpochMilli() }.getOrNull() }

        fun commandEvent(d: JsonObject, offsetMs: Long): JsonObjectBuilder.() -> Unit = {
            put("command", d.str("Command")?.lowercase() ?: "")
            put("positionMs", ticksToMs(d.long("PositionTicks") ?: 0))
            put("atMs", (parseInstantMs(d.str("When")) ?: System.currentTimeMillis().plus(offsetMs)) - offsetMs)
            put("playlistItemId", d.str("PlaylistItemId"))
        }

        fun queueEvent(d: JsonObject, src: String): JsonObjectBuilder.() -> Unit = {
            val list = (d["Playlist"] as? JsonArray).orEmpty().mapNotNull { it as? JsonObject }
            put("ids", JsonArray(list.mapNotNull { it.str("ItemId")?.let { id -> JsonPrimitive("$src:$id") } }))
            put("playlistItemIds", JsonArray(list.mapNotNull { it.str("PlaylistItemId")?.let(::JsonPrimitive) }))
            put("index", d.int("PlayingItemIndex") ?: 0)
            put("positionMs", ticksToMs(d.long("StartPositionTicks") ?: 0))
            put("playing", (d["IsPlaying"] as? JsonPrimitive)?.booleanOrNull ?: false)
        }

        private fun JsonObject.strings(k: String) = (get(k) as? JsonArray).orEmpty().mapNotNull { it.str() }
    }
}
