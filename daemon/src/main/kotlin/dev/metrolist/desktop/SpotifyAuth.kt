package dev.metrolist.desktop

import com.music.bitchord.data.Http
import com.music.bitchord.data.spotify.SpotifyLibrary
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.booleanOrNull
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
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/**
 * Spotify web-player token from the user's sp_dc cookie: the same flow BitChord's extension uses
 * (TOTP secret and query hashes are read from the live web-player bundle). Pathfinder reads
 * (playlists, tracks) are BitChord's own SpotifyLibrary, compiled from vendor/.
 */
object SpotifyAuth {
    private const val WEB = "https://open.spotify.com"
    private const val UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36"
    private const val PROFILE_HASH_FALLBACK = "08ffb4730af3746e04a8301396f20875dbbce10c75243803091a9274eacc8ac0"
    private val json = Json { ignoreUnknownKeys = true }

    private class WebPlayer(val secretVersion: Int, val secretCodes: List<Int>, val profileHash: String, val at: Long)
    private class Token(val value: String, val expiresAt: Long)

    @Volatile private var spDc: String? = null
    @Volatile private var token: Token? = null
    @Volatile private var player: WebPlayer? = null

    init {
        SpotifyLibrary.auth = SpotifyLibrary.Auth { accessToken()?.let { SpotifyLibrary.Tokens(it, null) } }
    }

    fun setCookie(value: String?) {
        spDc = value?.takeIf { it.isNotBlank() }
        token = null
    }

    private fun get(url: String, cookie: String? = null): okhttp3.Response =
        Http.client.newCall(
            Request.Builder().url(url).header("User-Agent", UA).apply { if (cookie != null) header("Cookie", "sp_dc=$cookie") }.build(),
        ).execute()

    private fun readWebPlayer(force: Boolean): WebPlayer {
        player?.takeIf { !force && System.currentTimeMillis() - it.at < 12 * 3600_000L }?.let { return it }
        val html = get("$WEB/").use { it.body.string() }
        val bundleUrl = Regex("""https://open\.spotifycdn\.com/cdn/build/web-player/web-player\.[0-9a-f]+\.js""").find(html)?.value
            ?: error("Spotify web player bundle not found")
        val bundle = get(bundleUrl).use { it.body.string() }
        val secret = listOf(Regex("""\{secret:'((?:\\.|[^'\\])*)',version:(\d+)\}"""), Regex("""\{secret:"((?:\\.|[^"\\])*)",version:(\d+)\}"""))
            .flatMap { re -> re.findAll(bundle).map { it.groupValues[2].toInt() to unescapeJs(it.groupValues[1]) }.toList() }
            .maxByOrNull { it.first } ?: error("Spotify TOTP secret not found")
        val profileHash = Regex("""["']profileAttributes["']\s*,\s*["']query["']\s*,\s*["']([0-9a-f]{64})["']""").find(bundle)?.groupValues?.get(1)
            ?: PROFILE_HASH_FALLBACK
        return WebPlayer(secret.first, secret.second.map { it.code }, profileHash, System.currentTimeMillis()).also { player = it }
    }

    private fun unescapeJs(s: String): String = Regex("""\\(u\{[0-9a-fA-F]+\}|u[0-9a-fA-F]{4}|x[0-9a-fA-F]{2}|[\s\S])""").replace(s) { m ->
        val e = m.groupValues[1]
        when (e[0]) {
            'u' -> String(Character.toChars(e.trim('u', '{', '}').toInt(16)))
            'x' -> e.substring(1).toInt(16).toChar().toString()
            'n' -> "\n"; 'r' -> "\r"; 't' -> "\t"; '0' -> "\u0000"
            else -> e
        }
    }

    internal fun totp(codes: List<Int>, epochMs: Long): String {
        val key = codes.mapIndexed { i, c -> c xor ((i % 33) + 9) }.joinToString("").toByteArray()
        val counter = java.nio.ByteBuffer.allocate(8).putLong(epochMs / 1000 / 30).array()
        val h = Mac.getInstance("HmacSHA1").apply { init(SecretKeySpec(key, "HmacSHA1")) }.doFinal(counter)
        val o = h.last().toInt() and 0x0f
        val bin = ((h[o].toInt() and 0x7f) shl 24) or ((h[o + 1].toInt() and 0xff) shl 16) or ((h[o + 2].toInt() and 0xff) shl 8) or (h[o + 3].toInt() and 0xff)
        return (bin % 1_000_000).toString().padStart(6, '0')
    }

    /** A signed-in bearer token, or null when signed out / the cookie is no longer accepted. */
    suspend fun accessToken(): String? = withContext(Dispatchers.IO) {
        val cookie = spDc ?: return@withContext null
        token?.takeIf { System.currentTimeMillis() < it.expiresAt - 30_000 }?.let { return@withContext it.value }
        for (force in listOf(false, true)) {
            val wp = readWebPlayer(force)
            val code = totp(wp.secretCodes, System.currentTimeMillis())
            val body = get("$WEB/api/token?reason=init&productType=web-player&totp=$code&totpServer=$code&totpVer=${wp.secretVersion}", cookie)
                .use { json.parseToJsonElement(it.body.string()).jsonObject }
            // The web player reads this flag to learn its secret was retired: reread the bundle once.
            if (body["totpVerExpired"] != null && !force) continue
            if (body["isAnonymous"]?.jsonPrimitive?.booleanOrNull != false) return@withContext null
            val t = body["accessToken"]?.jsonPrimitive?.contentOrNull ?: return@withContext null
            val exp = body["accessTokenExpirationTimestampMs"]?.jsonPrimitive?.longOrNull ?: (System.currentTimeMillis() + 3_600_000)
            token = Token(t, exp)
            return@withContext t
        }
        null
    }

    suspend fun status(): ServiceStatus {
        if (spDc == null) return ServiceStatus(false)
        return runCatching {
            val bearer = accessToken() ?: return ServiceStatus(false, error = "Spotify session rejected; sign in again")
            val hash = readWebPlayer(false).profileHash
            val payload = buildJsonObject {
                put("variables", buildJsonObject {})
                put("operationName", "profileAttributes")
                put("extensions", buildJsonObject { put("persistedQuery", buildJsonObject { put("version", 1); put("sha256Hash", hash) }) })
            }
            val profile = withContext(Dispatchers.IO) {
                Http.client.newCall(
                    Request.Builder().url("https://api-partner.spotify.com/pathfinder/v2/query")
                        .header("Authorization", "Bearer $bearer").header("App-Platform", "WebPlayer").header("User-Agent", UA)
                        .post(payload.toString().toRequestBody("application/json".toMediaType())).build(),
                ).execute().use { json.parseToJsonElement(it.body.string()).jsonObject }
            }["data"]?.jsonObject?.get("me")?.jsonObject?.get("profile")?.jsonObject
            val name = profile?.get("name")?.jsonPrimitive?.contentOrNull ?: profile?.get("username")?.jsonPrimitive?.contentOrNull ?: "Spotify"
            val avatar = profile?.get("avatar")?.jsonObject?.get("sources")?.jsonArray
                ?.maxByOrNull { it.jsonObject["width"]?.jsonPrimitive?.longOrNull ?: 0 }?.jsonObject?.get("url")?.jsonPrimitive?.contentOrNull
            ServiceStatus(true, UserDto(name, null, avatar))
        }.getOrElse { ServiceStatus(false, error = it.message ?: it::class.simpleName) }
    }
}
