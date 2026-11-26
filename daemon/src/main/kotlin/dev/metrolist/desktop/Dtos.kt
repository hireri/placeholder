package dev.metrolist.desktop

import kotlinx.serialization.Serializable

@Serializable data class UserDto(val name: String, val subtitle: String? = null, val avatarUrl: String? = null)

@Serializable data class ServiceStatus(val loggedIn: Boolean, val user: UserDto? = null, val error: String? = null)

@Serializable data class AuthStatus(val youtube: ServiceStatus, val spotify: ServiceStatus)

@Serializable data class YouTubeCreds(val cookie: String, val visitorData: String? = null, val dataSyncId: String? = null)

@Serializable data class SpotifyCreds(val spDc: String)

@Serializable data class StoredCreds(val youtube: YouTubeCreds? = null, val spotify: SpotifyCreds? = null, val lastfm: LastFmCreds? = null, val lastfmApp: LastFmAppKey? = null)

@Serializable
data class TrackDto(
    val id: String,
    val title: String,
    val artists: List<String>,
    val album: String? = null,
    val durationSec: Int? = null,
    val thumbnail: String? = null,
    val source: String,
    val artistId: String? = null,
    val albumId: String? = null,
    val explicit: Boolean = false,
    val video: Boolean = false,
)

@Serializable
data class PlaylistDto(val id: String, val title: String, val owner: String? = null, val thumbnail: String? = null, val source: String)

/** `track` is null when no YouTube Music song matches the Spotify metadata. */
@Serializable data class MatchDto(val track: TrackDto? = null, val retry: Boolean = false)

@Serializable
data class StreamDto(
    val url: String,
    val headers: Map<String, String>,
    val mimeType: String,
    val expiresInSeconds: Int,
    val client: String,
    val requireBoundedRange: Boolean,
    val rangeChunkSizeBytes: Long,
    val useRangeChunks: Boolean,
    /** Track length from the player response; catalogue entries for videos often carry none. */
    val durationSec: Int? = null,
    val bitrate: Int? = null,
    /** Measured loudness in LUFS as upstream normalization computes it (perceptual, else loudnessDb shifted by the AGGRESSIVE target). */
    val loudnessLufs: Double? = null,
)

@Serializable data class LyricsDto(val provider: String, val lrc: String)

/** One card/row of any kind; `type` is song | album | playlist | artist. `id` is the playable id for songs, the browse id otherwise. */
@Serializable
data class ItemDto(
    val type: String,
    val id: String,
    val title: String,
    val subtitle: String? = null,
    val thumbnail: String? = null,
    val durationSec: Int? = null,
    val artistId: String? = null,
    /** Songs only: the album they sit on, so a song card can open it. Null for singles. */
    val album: String? = null,
    val albumId: String? = null,
    val explicit: Boolean = false,
    val video: Boolean = false,
    /** Playlists only: owned by the signed-in user, so it can be deleted rather than just unsaved. */
    val editable: Boolean = false,
)

@Serializable data class SectionDto(val title: String, val items: List<ItemDto>)

/** Search-as-you-type: query completions plus the songs, albums and artists YouTube Music suggests for them. */
@Serializable data class SuggestDto(val queries: List<String>, val items: List<ItemDto>)

@Serializable data class PageDto(val item: ItemDto, val description: String? = null, val tracks: List<TrackDto> = emptyList(), val sections: List<SectionDto> = emptyList(), val channelId: String? = null, val subscribed: Boolean = false, val monthlyListeners: String? = null)

@Serializable
data class CreatePlaylistReq(val title: String, val videoIds: List<String> = emptyList())

@Serializable
data class CreatedPlaylistDto(val id: String, val added: Int, val total: Int)
