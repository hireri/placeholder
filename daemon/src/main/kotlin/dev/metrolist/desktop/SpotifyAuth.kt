package dev.metrolist.desktop

import com.music.bitchord.data.Http
import com.music.bitchord.data.spotify.SpotifyLibrary
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit

/** librespot's reusable login, as the sign-in helper made it. `credentials` is passed back to the helper unread. */
@Serializable data class SpotifyLogin(val deviceId: String, val credentials: JsonObject)

/**
 * Spotify's web-player API ("pathfinder") with tokens minted by `serafin-login spotify-token` from the saved librespot
 * login. The login is only reported as lost when Spotify refuses it; a network or rate-limit failure keeps it.
 */
object SpotifyAuth {
    private const val PROFILE_HASH = "08ffb4730af3746e04a8301396f20875dbbce10c75243803091a9274eacc8ac0"
    private const val GQL = "https://api-partner.spotify.com/pathfinder/v2/query"
    private const val UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36"
    /** The helper's exit code for a login Spotify no longer accepts. */
    private const val REJECTED = 2
    private const val UNREACHABLE = "Couldn't reach Spotify. Try again in a moment."
    private val json = Json { ignoreUnknownKeys = true }

    private class Token(val login: SpotifyLogin, val tokens: SpotifyLibrary.Tokens, val expiresAt: Long)

    @Volatile private var login: SpotifyLogin? = null
    @Volatile private var refused: SpotifyLogin? = null
    @Volatile private var cached: Token? = null
    private val lock = Mutex()
    private var mintFailedAt = 0L
    internal var retryAfterMs = 30_000L

    // Browsing is unofficial, so it stays well under anything a person clicking around would do: one request a second
    // on average (a page's few queries may go together), answers reused, and nothing sent while Spotify says to wait
    // (older answers are shown meanwhile).
    private val gate = RateGate(permits = 3, spacingMs = 1_000, backoffBaseMs = 3_000, maxBackoffMs = 300_000, burst = 5)
    @Volatile private var blockedUntil = 0L
    @Volatile private var strikes = 0
    private val answers = ConcurrentHashMap<String, Pair<Long, Any>>()
    private const val KEEP_MS = 5 * 60_000L

    /** Runs the helper with the login: its exit code and standard output. Replaced in tests. */
    internal var mint: suspend (SpotifyLogin) -> Pair<Int, String> = ::runHelper

    init {
        SpotifyLibrary.auth = SpotifyLibrary.Auth { tokens() }
    }

    fun set(value: SpotifyLogin?) {
        login = value
        refused = null
        cached = null
        mintFailedAt = 0
        blockedUntil = 0
        strikes = 0
        answers.clear()
    }

    val signedIn: Boolean get() = login != null && refused !== login

    /** Null when signed out or the login was refused; throws when Spotify cannot be reached right now. */
    suspend fun tokens(): SpotifyLibrary.Tokens? = lock.withLock {
        val current = login ?: return null
        if (refused === current) return null
        cached?.takeIf { it.login === current && System.currentTimeMillis() < it.expiresAt - 60_000 }?.let { return it.tokens }
        // Offline: don't start the helper again for every request that arrives meanwhile.
        check(System.currentTimeMillis() - mintFailedAt >= retryAfterMs) { UNREACHABLE }
        val (code, out) = mint(current)
        // Signed out or in again while the helper ran: its answer belongs to the old login.
        if (login !== current) return null
        if (code == REJECTED) {
            refused = current
            return null
        }
        if (code != 0) {
            mintFailedAt = System.currentTimeMillis()
            error(UNREACHABLE)
        }
        val body = json.parseToJsonElement(out).jsonObject
        val tokens = SpotifyLibrary.Tokens(body.getValue("bearer").jsonPrimitive.content, body["clientToken"]?.jsonPrimitive?.contentOrNull)
        cached = Token(current, tokens, System.currentTimeMillis() + (body["expiresIn"]?.jsonPrimitive?.longOrNull ?: 3600) * 1000)
        tokens
    }

    private suspend fun runHelper(login: SpotifyLogin): Pair<Int, String> = withContext(Dispatchers.IO) {
        val helper = System.getenv("SERAFIN_LOGIN_HELPER")?.takeIf { File(it).isFile } ?: error("The Spotify sign-in helper (serafin-login) is missing")
        val process = ProcessBuilder(helper, "spotify-token").redirectError(ProcessBuilder.Redirect.INHERIT)
            .apply { environment()["SPOTIFY_LOGIN"] = json.encodeToString(SpotifyLogin.serializer(), login) }.start()
        // The helper gives up by itself after 20 s; this only covers one that hangs.
        if (!process.waitFor(30, TimeUnit.SECONDS)) {
            process.destroyForcibly()
            return@withContext 1 to ""
        }
        process.exitValue() to process.inputStream.bufferedReader().readText()
    }

    /** Every call to Spotify goes through here: spaced out, remembered for a while, and stopped while rate limited. */
    @Suppress("UNCHECKED_CAST")
    suspend fun <T : Any> gated(key: String, keepMs: Long = KEEP_MS, block: suspend () -> T): T {
        val old = answers[key]
        if (old != null && System.currentTimeMillis() - old.first < keepMs) return old.second as T
        val wait = (blockedUntil - System.currentTimeMillis()) / 1000
        if (wait > 0) return old?.second as T? ?: error("Spotify asked Serafin to slow down. Try again in ${wait + 1} s.")
        val value = gate.run { runCatching { block() } }.getOrElse { if (it is CancellationException || old == null) throw it else return old.second as T }
        strikes = 0
        if (answers.size > 200) answers.clear()
        answers[key] = System.currentTimeMillis() to value
        return value
    }

    /** A 429: wait as long as Spotify says, else 30 s doubling to five minutes. Returns the seconds waited out. */
    internal fun rateLimited(retryAfter: Long?): Long {
        val seconds = retryAfter ?: minOf(30L shl minOf(strikes, 4), 300L)
        strikes++
        blockedUntil = System.currentTimeMillis() + seconds * 1000
        return seconds
    }

    /** One pathfinder query with the current tokens; returns its `data`. */
    suspend fun query(operation: String, hash: String, variables: JsonObject): JsonObject = gated("$operation $variables", keepFor(operation)) {
        val tokens = tokens() ?: error("Sign in to Spotify under Settings")
        val payload = buildJsonObject {
            put("variables", variables)
            put("operationName", operation)
            put("extensions", buildJsonObject { put("persistedQuery", buildJsonObject { put("version", 1); put("sha256Hash", hash) }) })
        }
        withContext(Dispatchers.IO) {
            val request = Request.Builder().url(GQL)
                .header("Authorization", "Bearer ${tokens.bearer}").header("app-platform", "WebPlayer")
                .header("Origin", "https://open.spotify.com").header("User-Agent", UA)
                .apply { tokens.clientToken?.let { header("Client-Token", it) } }
                .post(payload.toString().toRequestBody("application/json".toMediaType())).build()
            Http.client.newCall(request).execute().use { response ->
                if (response.code == 429) rateLimited(response.header("Retry-After")?.toLongOrNull())
                check(response.isSuccessful) { "Spotify $operation failed (${response.code})" }
                val root = json.parseToJsonElement(response.body.string()).jsonObject
                (root["data"] as? JsonObject) ?: error("Spotify $operation: ${root["errors"]?.jsonArray?.firstOrNull() ?: "no data"}")
            }
        }
    }

    /** The catalogue barely changes; a person's own library and playlists do. */
    private fun keepFor(operation: String) = when (operation) {
        "getAlbum" -> 6 * 60 * 60_000L
        "queryArtistOverview", "queryArtistDiscographyAll" -> 60 * 60_000L
        "homeSection", "queryWhatsNewFeed" -> 15 * 60_000L
        else -> KEEP_MS
    }

    suspend fun status(): ServiceStatus {
        if (login == null) return ServiceStatus(false)
        return try {
            if (tokens() == null) return ServiceStatus(false, error = "Sign in to Spotify again")
            val profile = query("profileAttributes", PROFILE_HASH, buildJsonObject {})["me"]?.jsonObject?.get("profile")?.jsonObject
            val name = profile?.get("name")?.jsonPrimitive?.contentOrNull ?: profile?.get("username")?.jsonPrimitive?.contentOrNull ?: "Spotify"
            val avatar = (profile?.get("avatar") as? JsonObject)?.get("sources")?.jsonArray
                ?.maxByOrNull { it.jsonObject["width"]?.jsonPrimitive?.longOrNull ?: 0 }?.jsonObject?.get("url")?.jsonPrimitive?.contentOrNull
            ServiceStatus(true, UserDto(name, null, avatar))
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // Still signed in: the saved login is fine, Spotify just can't be reached.
            ServiceStatus(true, error = e.message ?: e::class.simpleName)
        }
    }
}
