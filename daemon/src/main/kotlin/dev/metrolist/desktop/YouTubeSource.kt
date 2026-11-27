package dev.metrolist.desktop

/** The built-in source: a thin wrapper over [Services], which stays the YouTube implementation. */
object YouTubeSource : MusicSource {
    override val config = SourceConfig(id = "yt", type = "youtube", name = "YouTube Music")
    override val kind = Kind.CATALOG
    override val caps = setOf(Cap.SEARCH, Cap.HOME, Cap.LIBRARY, Cap.LIKE, Cap.PLAYLIST_WRITE, Cap.RADIO, Cap.VIDEO, Cap.HISTORY, Cap.SCROBBLE, Cap.IMAGE)

    override suspend fun test(): TestResultDto =
        runCatching { YouTubeAuth.status(Sources.store.load().youtube) }
            .map { TestResultDto(true, detail = it.user?.name ?: "Not signed in") }
            .getOrElse { TestResultDto(false, error = it.message) }

    override suspend fun search(q: String, type: String) = Services.searchItems(q, type)
    override suspend fun stream(id: String) = Services.stream(id)
    override suspend fun home() = Services.home().map { it.copy(shelf = it.shelf ?: ytShelf(it.title)) }
    override suspend fun library(kind: String) = Services.library(kind)
    override suspend fun album(id: String) = Services.album(id)
    override suspend fun artist(id: String) = Services.artist(id)
    override suspend fun playlist(id: String) = Services.playlist(id)
    override suspend fun radio(id: String) = Services.radio(id)
    override suspend fun like(id: String, on: Boolean) = Services.like(id, on)
    override suspend fun createPlaylist(title: String, trackIds: List<String>) = Services.createPlaylist(title, trackIds).id
    override suspend fun renamePlaylist(id: String, title: String) = Services.renamePlaylist(id, title)
    override suspend fun deletePlaylist(id: String) = Services.deletePlaylist(id)
    override suspend fun addToPlaylist(id: String, trackIds: List<String>) = Services.addToPlaylist(id, trackIds)
    override suspend fun removeFromPlaylist(id: String, trackIds: List<String>) = Services.removeFromPlaylist(id, trackIds)
    override suspend fun setImage(kind: String, id: String, bytes: ByteArray, mime: String) {
        check(kind == "playlist") { "YouTube Music only lets you change a playlist's image" }
        Services.setPlaylistImage(id, bytes)
    }

    override suspend fun reportPlay(id: String, state: String, pos: Double) { if (state == "finish") Services.registerPlayback(id) }
}

/** Maps a YouTube Home section title onto a typed shelf so it can mix with other sources' shelves; null stays untyped. */
internal fun ytShelf(title: String): String? {
    val t = title.lowercase()
    fun has(vararg k: String) = k.any { it in t }
    return when {
        has("listen again") -> "continue"
        has("favorites") -> "played"
        has("new release", "new album") -> "recent"
        has("mix", "picks", "for you", "made for") -> "mixes"
        has("trending", "chart", "top ", "popular", "discover") -> "discover"
        else -> null
    }
}
