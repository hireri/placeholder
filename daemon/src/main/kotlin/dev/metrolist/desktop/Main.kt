package dev.metrolist.desktop

import com.metrolist.music.lyrics.LyricsUtils
import io.ktor.http.ContentType
import io.ktor.http.HttpStatusCode
import io.ktor.serialization.kotlinx.json.json
import io.ktor.server.application.ApplicationCall
import io.ktor.server.application.call
import io.ktor.server.application.createApplicationPlugin
import io.ktor.server.application.install
import io.ktor.server.cio.CIO
import io.ktor.server.engine.embeddedServer
import io.ktor.server.plugins.contentnegotiation.ContentNegotiation
import io.ktor.server.plugins.statuspages.StatusPages
import io.ktor.server.request.receive
import io.ktor.server.response.respond
import io.ktor.server.response.respondBytesWriter
import io.ktor.server.response.respondText
import io.ktor.utils.io.writeFully
import io.ktor.server.routing.delete
import io.ktor.server.routing.get
import io.ktor.server.routing.post
import io.ktor.server.routing.put
import io.ktor.server.routing.routing
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.security.SecureRandom
import java.util.Base64

@Serializable private data class ErrorDto(val error: String)

private val creds = Sources.store

private suspend fun status(): AuthStatus {
    val stored = creds.load()
    return AuthStatus(YouTubeAuth.status(stored.youtube), SpotifyAuth.status())
}

private fun ApplicationCall.q(name: String) = request.queryParameters[name]

private val TokenGuard = { token: String ->
    createApplicationPlugin("TokenGuard") {
        onCall { call ->
            if (call.request.local.uri != "/health" && call.request.headers["X-Metrolist-Token"] != token) {
                call.respond(HttpStatusCode.Unauthorized, ErrorDto("bad token"))
            }
        }
    }
}

/** A bad value or a refused login is the caller's to fix (400), not a daemon failure. */
private suspend fun lastFmCall(call: ApplicationCall, block: suspend () -> LastFmDto) {
    try {
        call.respond(block())
    } catch (e: IllegalArgumentException) {
        call.respond(HttpStatusCode.BadRequest, ErrorDto(e.message ?: "invalid request"))
    }
}

fun main() {
    // Handshake: the client picks no port and no secret, the daemon reports both on stdout.
    val token = ByteArray(24).also(SecureRandom()::nextBytes).let { Base64.getUrlEncoder().withoutPadding().encodeToString(it) }

    runBlocking {
        val stored = creds.load()
        SpotifyAuth.setCookie(stored.spotify?.spDc)
        runCatching { YouTubeAuth.apply(stored.youtube) }
        runCatching { Settings.apply() }
        LastFm.init()
    }

    val server = embeddedServer(CIO, host = "127.0.0.1", port = System.getenv("METROLIST_PORT")?.toInt() ?: 0) {
        install(ContentNegotiation) { json(Json { ignoreUnknownKeys = true; encodeDefaults = true }) }
        install(StatusPages) {
            exception<IllegalArgumentException> { call, e -> call.respond(HttpStatusCode.BadRequest, ErrorDto(e.message ?: "invalid request")) }
            exception<UnsupportedOperationException> { call, e -> call.respond(HttpStatusCode.NotImplemented, ErrorDto(e.message ?: "unsupported")) }
            exception<Throwable> { call, e -> e.printStackTrace(); call.respond(HttpStatusCode.BadGateway, ErrorDto(e.message ?: e::class.simpleName ?: "error")) }
        }
        install(TokenGuard(token))

        routing {
            // A turned-off YouTube Music makes no Google calls: its legacy routes are refused before they run.
            intercept(io.ktor.server.application.ApplicationCallPipeline.Plugins) {
                val p = call.request.local.uri
                if ((p.startsWith("/youtube/") || p.startsWith("/stream/") || p.startsWith("/video/")) && !creds.load().youtubeEnabled) {
                    call.respond(HttpStatusCode.Conflict, ErrorDto("YouTube Music is turned off"))
                    finish()
                }
            }
            get("/health") { call.respondText("ok") }

            sourceRoutes()

            get("/settings") { call.respond(Settings.get()) }
            put("/settings") {
                val req = call.receive<SettingsDto>()
                try {
                    call.respond(Settings.update(req))
                } catch (e: IllegalArgumentException) {
                    call.respond(HttpStatusCode.BadRequest, ErrorDto(e.message ?: "invalid settings"))
                }
            }

            get("/lastfm") { call.respond(LastFm.get()) }
            put("/lastfm/key") { lastFmCall(call) { LastFm.setKey(call.receive()) } }
            put("/lastfm") { lastFmCall(call) { LastFm.update(call.receive<LastFmSettingsReq>()) } }
            post("/lastfm/login") { lastFmCall(call) { call.receive<LastFmLoginReq>().let { LastFm.login(it.username, it.password) } } }
            post("/lastfm/auth/start") { try { call.respond(LastFm.start()) } catch (e: IllegalArgumentException) { call.respond(HttpStatusCode.BadRequest, ErrorDto(e.message ?: "invalid request")) } }
            post("/lastfm/auth/finish") { lastFmCall(call) { LastFm.finish() } }
            delete("/lastfm/login") { call.respond(LastFm.logout()) }
            post("/lastfm/now-playing") { call.respond(LastFm.nowPlaying(call.receive())) }
            post("/lastfm/scrobble") { call.respond(LastFm.scrobble(call.receive())) }
            post("/lastfm/love") { call.respond(LastFm.love(call.receive())) }

            get("/auth/status") { call.respond(status()) }
            post("/auth/youtube") {
                val c = call.receive<YouTubeCreds>()
                creds.update { it.copy(youtube = c) }
                YouTubeAuth.apply(c)
                call.respond(status().youtube)
            }
            post("/auth/spotify") {
                val c = call.receive<SpotifyCreds>()
                creds.update { it.copy(spotify = c) }
                SpotifyAuth.setCookie(c.spDc)
                call.respond(SpotifyAuth.status())
            }
            delete("/auth/youtube") { creds.update { it.copy(youtube = null) }; YouTubeAuth.apply(null); call.respond(ServiceStatus(false)) }
            delete("/auth/spotify") { creds.update { it.copy(spotify = null) }; SpotifyAuth.setCookie(null); call.respond(ServiceStatus(false)) }

            get("/youtube/search") { call.respond(Services.search(call.q("q") ?: "")) }
            get("/youtube/match") {
                val result = Services.matchTrack(call.q("id"), call.q("title") ?: "", call.q("artist") ?: "", call.q("duration")?.toIntOrNull(), call.q("album"))
                call.respond(MatchDto(result.getOrNull(), retry = result.isFailure))
            }
            post("/youtube/playlists") {
                val req = call.receive<CreatePlaylistReq>()
                call.respond(Services.createPlaylist(req.title, req.videoIds))
            }
            delete("/youtube/playlists/{id}") { Services.deletePlaylist(call.parameters["id"]!!); call.respond(HttpStatusCode.NoContent) }
            get("/youtube/song/{videoId}") { call.respond(Services.song(call.parameters["videoId"]!!)) }
            get("/youtube/radio/{videoId}") { call.respond(Services.radio(call.parameters["videoId"]!!)) }
            get("/youtube/home") { call.respond(Services.home()) }
            get("/youtube/search/items") { call.respond(Services.searchItems(call.q("q") ?: "", call.q("type") ?: "song")) }
            get("/youtube/suggest") { call.respond(Services.suggest(call.q("q") ?: "")) }
            get("/youtube/explore") { call.respond(Services.explore()) }
            post("/youtube/like/{videoId}") { Services.like(call.parameters["videoId"]!!, call.q("on") == "true"); call.respond(HttpStatusCode.NoContent) }
            post("/youtube/subscribe/{channelId}") { Services.subscribe(call.parameters["channelId"]!!, call.q("on") == "true"); call.respond(HttpStatusCode.NoContent) }
            post("/youtube/save/{kind}/{id}") { Services.save(call.parameters["kind"]!!, call.parameters["id"]!!, call.q("on") == "true"); call.respond(HttpStatusCode.NoContent) }
            post("/youtube/playback/{videoId}") { Services.registerPlayback(call.parameters["videoId"]!!); call.respond(HttpStatusCode.NoContent) }
            get("/youtube/history") { call.respond(Services.history()) }
            get("/youtube/library/{kind}") { call.respond(Services.library(call.parameters["kind"]!!)) }
            get("/youtube/albums/{id}") { call.respond(Services.album(call.parameters["id"]!!)) }
            get("/youtube/artists/{id}") { call.respond(Services.artist(call.parameters["id"]!!)) }
            get("/youtube/playlists") { call.respond(Services.youtubePlaylists()) }
            get("/youtube/playlists/{id}/tracks") { call.respond(Services.youtubePlaylistTracks(call.parameters["id"]!!)) }
            get("/youtube/playlists/{id}") { call.respond(Services.playlist(call.parameters["id"]!!)) }
            get("/stream/{videoId}") { call.respond(Services.stream(call.parameters["videoId"]!!)) }
            get("/video/{videoId}") {
                val b = Services.videoBootstrap(call.parameters["videoId"]!!, call.q("maxHeight")?.toIntOrNull() ?: 480)
                val type = ContentType.parse(b.selectedVideoMimeType?.substringBefore(';') ?: "video/webm")
                call.respondBytesWriter(type) { Services.videoBytes(b, call.q("startMs")?.toLongOrNull() ?: 0).collect { writeFully(it) } }
            }
            get("/lyrics") {
                val l = Services.lyrics(call.q("id") ?: "", call.q("title") ?: "", call.q("artist") ?: "", call.q("duration")?.toIntOrNull() ?: -1, call.q("album"))
                if (l == null) call.respond(HttpStatusCode.NotFound, ErrorDto("no lyrics")) else call.respond(l)
            }

            post("/romanize") {
                call.respond(call.receive<List<String>>().map { if (LyricsUtils.isJapanese(it) && !LyricsUtils.isChinese(it)) LyricsUtils.romanizeJapanese(it) else "" })
            }

            get("/spotify/playlists") { call.respond(Services.spotifyPlaylists()) }
            get("/spotify/playlists/{id}/tracks") { call.respond(Services.spotifyTracks(call.parameters["id"]!!)) }
        }
    }
    server.start(wait = false)
    val port = runBlocking { server.engine.resolvedConnectors().first().port }
    Hls.base = "http://127.0.0.1:$port"; Hls.token = token
    println("""{"port":$port,"token":"$token"}""")
    System.out.flush()
    if (System.getenv("SERAFIN_EXIT_ON_STDIN_EOF") != null) {
        // The client holds our stdin; however it dies, the OS closes the pipe. Exit then, helpers included, instead of lingering.
        kotlin.concurrent.thread(isDaemon = true) {
            System.`in`.use { while (it.read() != -1) Unit }
            ProcessHandle.current().descendants().forEach { it.destroy() }
            kotlin.system.exitProcess(0)
        }
    }
    Thread.currentThread().join()
}
