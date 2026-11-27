package dev.metrolist.desktop

import io.ktor.http.HttpStatusCode
import io.ktor.server.application.call
import io.ktor.http.ContentType
import io.ktor.server.request.contentType
import io.ktor.server.request.receive
import io.ktor.server.response.respond
import io.ktor.server.routing.Route
import io.ktor.server.routing.delete
import io.ktor.server.routing.get
import io.ktor.server.routing.post
import io.ktor.server.routing.put
import kotlinx.serialization.Serializable

@Serializable private data class SourceError(val error: String)
@Serializable private data class TokenReq(val token: String)

/**
 * `/sources` routes manage the configured sources; `/source/{src}` routes browse one source; `/music/{op}/{id}` act on a namespaced item id
 * (see [Sources.parse]) so the client never needs to know which source an id belongs to.
 */
fun Route.sourceRoutes() {
    /** Browser sign-in for SoundCloud: the login helper posts the `oauth_token` cookie here; DELETE signs out. */
    post("/auth/soundcloud") {
        val cfg = Sources.configs().firstOrNull { it.type == "soundcloud" } ?: return@post call.respond(HttpStatusCode.NotFound, SourceError("no SoundCloud source"))
        val token = call.receive<TokenReq>().token.trim()
        if (token.isEmpty()) return@post call.respond(HttpStatusCode.BadRequest, SourceError("empty token"))
        Sources.patch(cfg.id, SourceReq(secret = token))
        call.respond(Sources.status(Sources.configs().first { it.id == cfg.id }))
    }
    delete("/auth/soundcloud") {
        Sources.configs().firstOrNull { it.type == "soundcloud" }?.let { Sources.patch(it.id, SourceReq(secret = "")) }
        call.respond(HttpStatusCode.NoContent)
    }
    get("/sources") { call.respond(Sources.configs().map(Sources::status)) }
    post("/sources") {
        try {
            val cfg = Sources.add(call.receive<SourceReq>())
            Sources.test(cfg.id)
            call.respond(Sources.status(cfg))
        } catch (e: IllegalArgumentException) { call.respond(HttpStatusCode.BadRequest, SourceError(e.message ?: "invalid source")) }
    }
    /** Dry run for the add-a-source form: nothing is stored. */
    post("/sources/test") {
        val req = call.receive<SourceReq>()
        val type = req.type?.lowercase()
        if (type == null || type !in Sources.types) return@post call.respond(HttpStatusCode.BadRequest, SourceError("unknown type"))
        val cfg = SourceConfig(id = "test", type = type, name = type, url = req.url?.trim()?.trimEnd('/'), username = req.username, secret = req.secret)
        call.respond(runCatching { Sources.build(cfg).test() }.getOrElse { TestResultDto(false, error = it.message) })
    }
    put("/sources/order") { Sources.reorder(call.receive<List<String>>()); call.respond(Sources.configs().map(Sources::status)) }
    put("/sources/{id}") {
        val id = call.parameters["id"]!!
        try {
            if (!Sources.exists(id)) return@put call.respond(HttpStatusCode.NotFound, SourceError("no such source"))
            val req = call.receive<SourceReq>()
            if (req.enabled == false) Mix.snapshot(id)
            Sources.patch(id, req)
            call.respond(Sources.status(Sources.configs().first { it.id == id }))
        } catch (e: IllegalArgumentException) { call.respond(HttpStatusCode.BadRequest, SourceError(e.message ?: "invalid")) }
    }
    delete("/sources/{id}") {
        try { Sources.remove(call.parameters["id"]!!); call.respond(HttpStatusCode.NoContent) }
        catch (e: IllegalArgumentException) { call.respond(HttpStatusCode.BadRequest, SourceError(e.message ?: "invalid")) }
    }
    post("/sources/{id}/test") { call.respond(Sources.test(call.parameters["id"]!!)) }

    get("/mix/home") { call.respond(Mix.home(call.request.queryParameters["layout"] ?: "mixed")) }
    get("/cover") {
        val q = call.request.queryParameters
        call.respond(CoverArt.lookup(q["artist"].orEmpty(), q["album"].orEmpty(), q["title"].orEmpty()))
    }
    get("/mix/search") { call.respond(Mix.songs(call.request.queryParameters["q"] ?: "")) }
    get("/mix/search/items") { call.respond(Mix.search(call.request.queryParameters["q"] ?: "", call.request.queryParameters["type"] ?: "song")) }
    get("/mix/suggest") { call.respond(Mix.suggest(call.request.queryParameters["q"] ?: "")) }
    get("/mix/library/{kind}") { call.respond(Mix.library(call.parameters["kind"]!!)) }

    get("/source/{src}/search") { val s = call.parameters["src"]!!; call.respond(Sources.use(s).search(call.request.queryParameters["q"] ?: "", call.request.queryParameters["type"] ?: "song").map { it.ns(s) }) }
    get("/source/{src}/home") { val s = call.parameters["src"]!!; call.respond(Sources.use(s).home().map { it.ns(s) }) }
    get("/source/{src}/library/{kind}") { val s = call.parameters["src"]!!; call.respond(Sources.use(s).library(call.parameters["kind"]!!).map { it.ns(s) }) }

    get("/hls/{key}") { Hls.serve(call, call.parameters["key"]!!) }
    // Only YouTube can look a song up by id; the others know songs through their albums and playlists.
    get("/music/song/{id}") { val (s, id) = Sources.parse(call.parameters["id"]!!); Sources.use(s); if (s != "yt") unsupported(); call.respond(Services.song(id)) }
    // Playlists live on one source: every song added must come from the playlist's own source.
    post("/music/playlist") {
        val req = call.receive<PlaylistCreateReq>()
        val src = Sources.use(req.source).also { requireWritable(it) }
        require(req.title.isNotBlank()) { "Give the playlist a name" }
        val ids = req.trackIds.map { sameSource(req.source, it) }
        call.respond(PlaylistCreatedDto(Sources.fullId(req.source, src.createPlaylist(req.title.trim(), ids))))
    }
    put("/music/playlist/{id}") {
        val (s, id) = Sources.parse(call.parameters["id"]!!)
        val title = call.receive<PlaylistRenameReq>().title.trim()
        require(title.isNotEmpty()) { "Give the playlist a name" }
        Sources.use(s).also { requireWritable(it) }.renamePlaylist(id, title)
        call.respond(HttpStatusCode.NoContent)
    }
    delete("/music/playlist/{id}") {
        val (s, id) = Sources.parse(call.parameters["id"]!!)
        Sources.use(s).also { requireWritable(it) }.deletePlaylist(id)
        call.respond(HttpStatusCode.NoContent)
    }
    post("/music/playlist/{id}/add") {
        val (s, id) = Sources.parse(call.parameters["id"]!!)
        val ids = call.receive<PlaylistTracksReq>().trackIds.map { sameSource(s, it) }
        Sources.use(s).also { requireWritable(it) }.addToPlaylist(id, ids)
        call.respond(HttpStatusCode.NoContent)
    }
    post("/music/playlist/{id}/remove") {
        val (s, id) = Sources.parse(call.parameters["id"]!!)
        val ids = call.receive<PlaylistTracksReq>().trackIds.map { sameSource(s, it) }
        Sources.use(s).also { requireWritable(it) }.removeFromPlaylist(id, ids)
        call.respond(HttpStatusCode.NoContent)
    }
    /** The image is the raw request body; `Content-Type` says which format. */
    put("/music/image/{kind}/{id}") {
        val (s, id) = Sources.parse(call.parameters["id"]!!)
        val mime = call.request.contentType().let { "${it.contentType}/${it.contentSubtype}" }
        require(mime in setOf("image/jpeg", "image/png", "image/webp")) { "Use a JPEG, PNG or WebP image" }
        val bytes = call.receive<ByteArray>()
        require(bytes.size in 1..MAX_IMAGE) { "The image must be under ${MAX_IMAGE / 1_000_000} MB" }
        val src = Sources.use(s)
        check(Cap.IMAGE in src.caps) { "${src.config.name} doesn't support changing images" }
        src.setImage(call.parameters["kind"]!!, id, bytes, mime)
        call.respond(HttpStatusCode.NoContent)
    }
    get("/music/stream/{id}") { val (s, id) = Sources.parse(call.parameters["id"]!!); call.respond(Sources.use(s).stream(id)) }
    get("/music/album/{id}") { val (s, id) = Sources.parse(call.parameters["id"]!!); call.respond(Sources.use(s).album(id).ns(s)) }
    get("/music/artist/{id}") { val (s, id) = Sources.parse(call.parameters["id"]!!); call.respond(Sources.use(s).artist(id).ns(s)) }
    get("/music/playlist/{id}") { val (s, id) = Sources.parse(call.parameters["id"]!!); call.respond(Sources.use(s).playlist(id).ns(s)) }
    get("/music/radio/{id}") { val (s, id) = Sources.parse(call.parameters["id"]!!); call.respond(Sources.use(s).radio(id).map { it.ns(s) }) }
    post("/music/like/{id}") { val (s, id) = Sources.parse(call.parameters["id"]!!); Sources.use(s).like(id, call.request.queryParameters["on"] == "true"); call.respond(HttpStatusCode.NoContent) }
    post("/music/played/{id}") { val (s, id) = Sources.parse(call.parameters["id"]!!); val q = call.request.queryParameters; Sources.use(s).reportPlay(id, q["state"] ?: if (q["finished"] == "true") "finish" else "stop", q["pos"]?.toDoubleOrNull() ?: 0.0); call.respond(HttpStatusCode.NoContent) }
}

private const val MAX_IMAGE = 10_000_000

private fun requireWritable(src: MusicSource) = check(Cap.PLAYLIST_WRITE in src.caps) { "${src.config.name} doesn't support editing playlists" }

/** The raw id of a song that must belong to source `src`; a song from elsewhere cannot go on its playlist. */
private fun sameSource(src: String, full: String): String {
    val (s, id) = Sources.parse(full)
    require(s == src) { "Songs from another source can't be added to a playlist on ${Sources.get(src).config.name}" }
    return id
}
