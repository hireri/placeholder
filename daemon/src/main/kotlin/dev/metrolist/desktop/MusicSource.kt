package dev.metrolist.desktop

import kotlinx.serialization.Serializable

enum class Kind { CATALOG, LIBRARY }

enum class Cap { SEARCH, HOME, LIBRARY, LIKE, PLAYLIST_WRITE, RADIO, VIDEO, HISTORY, SCROBBLE, IMAGE }

/** Persisted in credentials.json. `secret` is the password or token, never sent back to the client. */
@Serializable
data class SourceConfig(
    val id: String,
    /** youtube | navidrome | jellyfin | soundcloud */
    val type: String,
    val name: String,
    val url: String? = null,
    val username: String? = null,
    val secret: String? = null,
    val enabled: Boolean = true,
    /** Who scrobbles this source's plays to Last.fm: "serafin" or "server" (the server already does it). */
    val scrobbleVia: String = "serafin",
    /** Include this source on the merged Home. */
    val home: Boolean = true,
    /** Include this source in search. */
    val search: Boolean = true,
)

@Serializable data class TestResultDto(val ok: Boolean, val error: String? = null, val detail: String? = null, val avatar: String? = null)

/** What the Sources settings page shows. Never contains the secret. */
@Serializable
data class SourceStatusDto(
    val id: String,
    val type: String,
    val name: String,
    val kind: String,
    val caps: List<String>,
    val enabled: Boolean,
    val builtin: Boolean,
    /** ok | error | unchecked (no test run since the daemon started) */
    val status: String,
    val error: String? = null,
    val detail: String? = null,
    val url: String? = null,
    val username: String? = null,
    val scrobbleVia: String,
    val home: Boolean,
    val search: Boolean = true,
    /** One per install (SoundCloud, Bandcamp): always listed, turned on or off, never added or removed. */
    val singleton: Boolean = false,
    /** A token or password is saved (never sent back). */
    val secretSet: Boolean = false,
    /** The signed-in account's picture, when the last test found one. */
    val avatarUrl: String? = null,
)

/** Create or patch. On create `type` is required; on patch every field is optional and only present ones change. */
@Serializable
data class SourceReq(
    val type: String? = null,
    val name: String? = null,
    val url: String? = null,
    val username: String? = null,
    val secret: String? = null,
    val enabled: Boolean? = null,
    val scrobbleVia: String? = null,
    val home: Boolean? = null,
    val search: Boolean? = null,
)

@Serializable data class PlaylistCreateReq(val source: String, val title: String, val trackIds: List<String> = emptyList())
@Serializable data class PlaylistRenameReq(val title: String)
@Serializable data class PlaylistTracksReq(val trackIds: List<String>)
@Serializable data class PlaylistCreatedDto(val id: String)

/**
 * One music provider. Implementations speak their own raw ids; [Sources] namespaces them (`<sourceId>:<rawId>`) on the way out
 * and strips the prefix on the way in, except for YouTube whose ids stay bare so existing client code keeps working.
 * Unsupported operations throw [UnsupportedOperationException] (the API turns that into 501); the UI hides them via [caps].
 *
 * Contract for non-YouTube [stream]: a plain progressive URL, `headers` empty unless the server needs them,
 * `requireBoundedRange = false`, `useRangeChunks = false`, `rangeChunkSizeBytes = 0`, `expiresInSeconds` large, `client = config.id`.
 * Every [TrackDto] and [ItemDto] returned has `source = config.id` (TrackDto) and a thumbnail that is directly fetchable by the client.
 */
interface MusicSource {
    val config: SourceConfig
    val kind: Kind
    val caps: Set<Cap>

    /** Cheap connectivity and credentials check. Must not throw: report failure in the result. */
    suspend fun test(): TestResultDto

    /** type: song | album | artist | playlist | video */
    suspend fun search(q: String, type: String): List<ItemDto>
    suspend fun stream(id: String): StreamDto

    /** Shelves; set [SectionDto.shelf] to continue | recent | mixes | discover | played so the client can place them. */
    suspend fun home(): List<SectionDto> = emptyList()
    /** kind: songs | albums | artists | playlists */
    suspend fun library(kind: String): List<ItemDto> = emptyList()
    suspend fun album(id: String): PageDto = unsupported()
    suspend fun artist(id: String): PageDto = unsupported()
    suspend fun playlist(id: String): PageDto = unsupported()
    suspend fun radio(id: String): List<TrackDto> = emptyList()
    suspend fun like(id: String, on: Boolean): Unit = unsupported()
    // Playlist and image writes, for sources with [Cap.PLAYLIST_WRITE] / [Cap.IMAGE]. Ids are the source's raw ids; a refusal
    // (permissions, an old server) throws with a message fit to show the user.
    /** Returns the new playlist's raw id. */
    suspend fun createPlaylist(title: String, trackIds: List<String>): String = unsupported()
    suspend fun renamePlaylist(id: String, title: String): Unit = unsupported()
    suspend fun deletePlaylist(id: String): Unit = unsupported()
    suspend fun addToPlaylist(id: String, trackIds: List<String>): Unit = unsupported()
    /** Removes every occurrence of the given songs; each source maps song ids to its own entry ids. */
    suspend fun removeFromPlaylist(id: String, trackIds: List<String>): Unit = unsupported()
    /** `kind` is playlist | album | artist; sources accept the kinds they can. `mime` is image/jpeg, image/png or image/webp. */
    suspend fun setImage(kind: String, id: String, bytes: ByteArray, mime: String): Unit = unsupported()

    /** LRC text (timestamped when the source has synced lines), or null when it has none or the lookup fails. */
    suspend fun lyrics(id: String): String? = null

    /** Tell the source a track started, stopped at [pos] seconds, or finished. Best effort; never scrobbles elsewhere. */
    suspend fun reportPlay(id: String, state: String, pos: Double) {}
}

fun unsupported(): Nothing = throw UnsupportedOperationException("not supported by this source")

fun lrcTimestamp(ms: Long) = "%02d:%02d.%02d".format(ms / 60_000, ms / 1000 % 60, ms / 10 % 100)
