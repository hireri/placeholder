package dev.metrolist.desktop

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
import io.ktor.server.routing.routing
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.security.SecureRandom
import java.util.Base64

@Serializable private data class ErrorDto(val error: String)

private val creds = CredentialStore()

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

fun main() {
    // Handshake: the client picks no port and no secret, the daemon reports both on stdout.
    val token = ByteArray(24).also(SecureRandom()::nextBytes).let { Base64.getUrlEncoder().withoutPadding().encodeToString(it) }

    runBlocking {
        val stored = creds.load()
        SpotifyAuth.setCookie(stored.spotify?.spDc)
        runCatching { YouTubeAuth.apply(stored.youtube) }
    }

    val server = embeddedServer(CIO, host = "127.0.0.1", port = System.getenv("METROLIST_PORT")?.toInt() ?: 0) {
        install(ContentNegotiation) { json(Json { ignoreUnknownKeys = true; encodeDefaults = true }) }
        install(StatusPages) {
            exception<Throwable> { call, e -> call.respond(HttpStatusCode.BadGateway, ErrorDto(e.message ?: e::class.simpleName ?: "error")) }
        }
        install(TokenGuard(token))

        routing {
            get("/health") { call.respondText("ok") }

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
            get("/youtube/home") { call.respond(Services.home()) }
            get("/youtube/search/items") { call.respond(Services.searchItems(call.q("q") ?: "", call.q("type") ?: "song")) }
            get("/youtube/explore") { call.respond(Services.explore()) }
            post("/youtube/like/{videoId}") { Services.like(call.parameters["videoId"]!!, call.q("on") == "true"); call.respond(HttpStatusCode.NoContent) }
            post("/youtube/subscribe/{channelId}") { Services.subscribe(call.parameters["channelId"]!!, call.q("on") == "true"); call.respond(HttpStatusCode.NoContent) }
            post("/youtube/save/{kind}/{id}") { Services.save(call.parameters["kind"]!!, call.parameters["id"]!!, call.q("on") == "true"); call.respond(HttpStatusCode.NoContent) }
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

            get("/spotify/playlists") { call.respond(Services.spotifyPlaylists()) }
            get("/spotify/playlists/{id}/tracks") { call.respond(Services.spotifyTracks(call.parameters["id"]!!)) }
        }
    }
    server.start(wait = false)
    val port = runBlocking { server.engine.resolvedConnectors().first().port }
    println("""{"port":$port,"token":"$token"}""")
    System.out.flush()
    Thread.currentThread().join()
}
