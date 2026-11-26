package dev.metrolist.desktop

import com.metrolist.lastfm.LastFM
import com.metrolist.music.constants.EnableLastFMScrobblingKey
import com.metrolist.music.constants.LastFMUseNowPlaying
import com.metrolist.music.constants.LastFMUseSendLikes
import com.metrolist.music.constants.ScrobbleDelayPercentKey
import com.metrolist.music.constants.ScrobbleDelaySecondsKey
import com.metrolist.music.constants.ScrobbleMinSongDurationKey
import com.metrolist.music.utils.DesktopPaths
import androidx.datastore.preferences.core.edit
import android.content.Context
import com.metrolist.music.utils.dataStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.net.URL
import java.net.URLEncoder
import kotlinx.serialization.Serializable
import java.io.File
import java.util.Properties

@Serializable data class LastFmAppKey(val apiKey: String, val secret: String)
@Serializable data class LastFmKeyReq(val apiKey: String = "", val secret: String = "")
@Serializable data class LastFmCreds(val session: String, val username: String)

/** Everything the client needs to drive scrobbling; `GET /lastfm` and the reply of every write. */
@Serializable
data class LastFmDto(
    val configured: Boolean = false,
    val customKey: Boolean = false,
    val loggedIn: Boolean = false,
    val username: String? = null,
    val avatarUrl: String? = null,
    val enabled: Boolean = false,
    val nowPlaying: Boolean = true,
    val sendLikes: Boolean = false,
    val delayPercent: Float = LastFM.DEFAULT_SCROBBLE_DELAY_PERCENT,
    val minDuration: Int = LastFM.DEFAULT_SCROBBLE_MIN_SONG_DURATION,
    val delaySeconds: Int = LastFM.DEFAULT_SCROBBLE_DELAY_SECONDS,
)

@Serializable data class LastFmSettingsReq(val enabled: Boolean? = null, val nowPlaying: Boolean? = null, val sendLikes: Boolean? = null, val delayPercent: Float? = null, val minDuration: Int? = null, val delaySeconds: Int? = null)

@Serializable data class LastFmAuthUrl(val url: String)
@Serializable data class LastFmLoginReq(val username: String, val password: String)

@Serializable data class LastFmTrackReq(val artist: String, val track: String, val album: String? = null, val duration: Int? = null, val timestamp: Long? = null, val love: Boolean? = null)

/**
 * Port of upstream's Last.fm integration over the unmodified `com.metrolist.lastfm.LastFM`. The play/pause timer of upstream's
 * ScrobbleManager lives in the client, which knows the playback state; these are the calls it makes and the stored settings.
 * The API key and secret are not in the sources (upstream injects them at build time): LASTFM_API_KEY / LASTFM_SECRET in the
 * environment, or the same names in `lastfm.properties` in the data directory (release builds also bundle one), or a key the user enters.
 */
object LastFm {
    private const val NO_KEY = "No Last.fm API key: set LASTFM_API_KEY and LASTFM_SECRET (see lastfm.properties in the data folder)"
    private val store = Context().dataStore
    private val creds = CredentialStore()

    fun init() {
        configure()
        LastFM.sessionKey = creds.load().lastfm?.session
    }

    /** The user's own key wins over the environment, which wins over the `lastfm.properties` the release bundles. */
    private fun configure() {
        val custom = creds.load().lastfmApp
        val props = Properties().apply { runCatching { File(DesktopPaths.dataDir, "lastfm.properties").inputStream().use(::load) } }
        // Release builds bake the project's key into the daemon jar (daemon/src/main/resources, written by CI, never committed).
        val bundled = Properties().apply { runCatching { LastFm::class.java.getResourceAsStream("/lastfm.properties")?.use(::load) } }
        fun key(name: String) = listOf(System.getenv(name), props.getProperty(name), bundled.getProperty(name)).firstOrNull { !it.isNullOrBlank() }.orEmpty().trim()
        apiKey = custom?.apiKey ?: key("LASTFM_API_KEY")
        LastFM.initialize(apiKey, custom?.secret ?: key("LASTFM_SECRET"))
    }

    /** Both blank clears the custom key. A session belongs to the key that made it, so changing the key signs out. */
    suspend fun setKey(r: LastFmKeyReq): LastFmDto {
        val (k, s) = r.apiKey.trim() to r.secret.trim()
        require((k.isEmpty() && s.isEmpty()) || (k.isNotEmpty() && s.isNotEmpty())) { "Enter both the API key and the secret" }
        creds.update { it.copy(lastfmApp = if (k.isEmpty()) null else LastFmAppKey(k, s), lastfm = null) }
        pending = null
        forgetAvatar()
        LastFM.sessionKey = null
        configure()
        return get()
    }

    @Volatile private var avatar: String? = null
    @Volatile private var avatarTried = false

    /** Largest profile picture Last.fm has for [user]; null when there is none or the lookup fails. */
    private fun fetchAvatar(user: String): String? = runCatching {
        val enc = { s: String -> URLEncoder.encode(s, "UTF-8") }
        val url = "https://ws.audioscrobbler.com/2.0/?method=user.getinfo&format=json&user=${enc(user)}&api_key=${enc(apiKey)}"
        val body = URL(url).openConnection().apply { connectTimeout = 5000; readTimeout = 5000; setRequestProperty("User-Agent", "Serafin") }.getInputStream().use { it.readBytes().decodeToString() }
        Json.parseToJsonElement(body).jsonObject["user"]?.jsonObject?.get("image")?.jsonArray
            ?.mapNotNull { it.jsonObject["#text"]?.jsonPrimitive?.contentOrNull?.takeIf(String::isNotBlank) }?.lastOrNull()
    }.onFailure { System.err.println("last.fm avatar: $it") }.getOrNull()

    private fun forgetAvatar() { avatar = null; avatarTried = false }

    suspend fun get(): LastFmDto {
        val p = store.data.first()
        val c = creds.load().lastfm
        if (c != null && !avatarTried) {
            avatarTried = true
            avatar = withContext(Dispatchers.IO) { fetchAvatar(c.username) }
        }
        return LastFmDto(
            configured = LastFM.isInitialized(),
            customKey = creds.load().lastfmApp != null,
            loggedIn = c != null,
            username = c?.username,
            avatarUrl = if (c != null) avatar else null,
            enabled = p[EnableLastFMScrobblingKey] ?: false,
            nowPlaying = p[LastFMUseNowPlaying] ?: true,
            sendLikes = p[LastFMUseSendLikes] ?: false,
            delayPercent = p[ScrobbleDelayPercentKey] ?: LastFM.DEFAULT_SCROBBLE_DELAY_PERCENT,
            minDuration = p[ScrobbleMinSongDurationKey] ?: LastFM.DEFAULT_SCROBBLE_MIN_SONG_DURATION,
            delaySeconds = p[ScrobbleDelaySecondsKey] ?: LastFM.DEFAULT_SCROBBLE_DELAY_SECONDS,
        )
    }

    suspend fun update(r: LastFmSettingsReq): LastFmDto {
        r.delayPercent?.let { require(it in 0.3f..0.95f) { "delayPercent must be between 0.3 and 0.95" } }
        r.minDuration?.let { require(it in 0..600) { "minDuration must be between 0 and 600" } }
        r.delaySeconds?.let { require(it in 30..600) { "delaySeconds must be between 30 and 600" } }
        store.edit { e ->
            r.enabled?.let { e[EnableLastFMScrobblingKey] = it }
            r.nowPlaying?.let { e[LastFMUseNowPlaying] = it }
            r.sendLikes?.let { e[LastFMUseSendLikes] = it }
            r.delayPercent?.let { e[ScrobbleDelayPercentKey] = it }
            r.minDuration?.let { e[ScrobbleMinSongDurationKey] = it }
            r.delaySeconds?.let { e[ScrobbleDelaySecondsKey] = it }
        }
        return get()
    }

    /** Throws IllegalArgumentException (shown to the user) when Last.fm refuses the login or no API key is set. */
    suspend fun login(username: String, password: String): LastFmDto {
        require(LastFM.isInitialized()) { NO_KEY }
        val auth = LastFM.getMobileSession(username, password).getOrElse { throw IllegalArgumentException(it.message ?: "Last.fm login failed") }
        creds.update { it.copy(lastfm = LastFmCreds(auth.session.key, auth.session.name)) }
        LastFM.sessionKey = auth.session.key
        forgetAvatar()
        return get()
    }

    private var apiKey = ""
    @Volatile private var pending: String? = null

    /** Web auth: a request token and the Last.fm page where the user approves it; [finish] completes the sign-in. */
    suspend fun start(): LastFmAuthUrl {
        require(LastFM.isInitialized()) { NO_KEY }
        val token = LastFM.getToken().getOrElse { throw IllegalArgumentException(it.message ?: "Last.fm is unreachable") }.token
        pending = token
        return LastFmAuthUrl("https://www.last.fm/api/auth/?api_key=$apiKey&token=$token")
    }

    /** Exchanges the pending token for a session; fails with a plain message until the user has approved it in the browser. */
    suspend fun finish(): LastFmDto {
        val token = requireNotNull(pending) { "Start the sign-in first" }
        val auth = LastFM.getSession(token).getOrElse { throw IllegalArgumentException("Last.fm has not confirmed the sign-in yet: approve it in your browser, then try again") }
        pending = null
        creds.update { it.copy(lastfm = LastFmCreds(auth.session.key, auth.session.name)) }
        LastFM.sessionKey = auth.session.key
        forgetAvatar()
        return get()
    }

    suspend fun logout(): LastFmDto {
        creds.update { it.copy(lastfm = null) }
        LastFM.sessionKey = null
        forgetAvatar()
        return get()
    }

    /** True when the call went out and Last.fm accepted it; false when the setting or the login says to skip. */
    private suspend fun send(gate: (LastFmDto) -> Boolean, call: suspend () -> Result<io.ktor.client.statement.HttpResponse>): Boolean {
        val s = get()
        if (!s.loggedIn || !s.enabled || !gate(s)) return false
        val r = call().getOrThrow()
        check(r.status.value in 200..299) { "Last.fm answered ${r.status.value}" }
        return true
    }

    suspend fun nowPlaying(r: LastFmTrackReq) = send({ it.nowPlaying }) { LastFM.updateNowPlaying(r.artist, r.track, r.album, duration = r.duration) }

    suspend fun scrobble(r: LastFmTrackReq) = send({ true }) { LastFM.scrobble(r.artist, r.track, r.timestamp ?: (System.currentTimeMillis() / 1000), r.album, duration = r.duration) }

    suspend fun love(r: LastFmTrackReq) = send({ it.sendLikes }) { LastFM.setLoveStatus(r.artist, r.track, r.love ?: true) }
}
