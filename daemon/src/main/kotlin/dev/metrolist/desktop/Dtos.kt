package dev.metrolist.desktop

import kotlinx.serialization.Serializable

@Serializable data class UserDto(val name: String, val subtitle: String? = null, val avatarUrl: String? = null)

@Serializable data class ServiceStatus(val loggedIn: Boolean, val user: UserDto? = null, val error: String? = null)

@Serializable data class AuthStatus(val youtube: ServiceStatus, val spotify: ServiceStatus)

@Serializable data class YouTubeCreds(val cookie: String, val visitorData: String? = null, val dataSyncId: String? = null)

@Serializable data class SpotifyCreds(val spDc: String)

@Serializable data class StoredCreds(val youtube: YouTubeCreds? = null, val spotify: SpotifyCreds? = null)

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
)

@Serializable
data class PlaylistDto(val id: String, val title: String, val owner: String? = null, val thumbnail: String? = null, val source: String)

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
)

@Serializable data class SectionDto(val title: String, val items: List<ItemDto>)

@Serializable data class PageDto(val item: ItemDto, val description: String? = null, val tracks: List<TrackDto> = emptyList(), val sections: List<SectionDto> = emptyList(), val channelId: String? = null, val subscribed: Boolean = false, val monthlyListeners: String? = null)
