package dev.metrolist.desktop

import android.content.Context
import android.net.ConnectivityManager
import com.metrolist.innertube.YouTube
import com.metrolist.innertube.models.AlbumItem
import com.metrolist.innertube.models.ArtistItem
import com.metrolist.innertube.models.YTItem
import com.metrolist.innertube.utils.completed
import com.metrolist.innertube.models.PlaylistItem
import com.metrolist.innertube.models.SongItem
import com.metrolist.music.constants.AudioQuality
import com.metrolist.music.lyrics.LyricsProviderRegistry
import com.metrolist.music.lyrics.LyricsUtils
import com.metrolist.music.utils.InnerTubeXPlayer
import com.music.bitchord.data.spotify.SpotifyLibrary
import kotlinx.coroutines.CancellationException

object Services {
    /** Upstream code wants a Context; on desktop it is the shim. */
    val context = Context()

    init {
        InnerTubeXPlayer.initialize(context)
    }

    suspend fun search(query: String): List<TrackDto> =
        YouTube.search(query, YouTube.SearchFilter.FILTER_SONG).getOrThrow().items.filterIsInstance<SongItem>().map { it.toDto() }

    suspend fun youtubePlaylists(): List<PlaylistDto> =
        YouTube.library("FEmusic_liked_playlists").getOrThrow().items.mapNotNull {
            when (it) {
                is PlaylistItem -> PlaylistDto(it.id, it.title, it.author?.name, it.thumbnail, "youtube")
                is AlbumItem -> PlaylistDto(it.playlistId, it.title, it.artists?.joinToString { a -> a.name }, it.thumbnail, "youtube")
                else -> null
            }
        }

    suspend fun youtubePlaylistTracks(id: String): List<TrackDto> =
        YouTube.playlist(id).getOrThrow().songs.map { it.toDto() }

    suspend fun spotifyPlaylists(): List<PlaylistDto> =
        SpotifyLibrary.playlists().map { PlaylistDto(it.id, it.name, it.owner, it.imageUrl, "spotify") }

    suspend fun spotifyTracks(id: String): List<TrackDto> =
        SpotifyLibrary.tracks(id).map {
            TrackDto(it.id, it.title, it.artist.split(", ").filter(String::isNotBlank), it.album, it.durationMs / 1000, it.imageUrl, "spotify")
        }

    /** The stream URL plus the headers the player must send with it (upstream InnerTubeXPlayer, unmodified). */
    suspend fun stream(videoId: String): StreamDto {
        val data = InnerTubeXPlayer.playerResponseForPlayback(videoId, null, AudioQuality.AUTO, ConnectivityManager()).getOrThrow()
        return StreamDto(
            url = data.streamUrl,
            headers = data.streamHeaders,
            mimeType = data.format.mimeType,
            expiresInSeconds = data.streamExpiresInSeconds,
            client = data.streamClient,
            requireBoundedRange = data.requireBoundedRange,
            rangeChunkSizeBytes = data.rangeChunkSizeBytes,
            useRangeChunks = data.useRangeChunks,
        )
    }

    /** First provider (in upstream's default order) that has lyrics. Upstream's LyricsHelper is DB-bound, so this is the thin desktop equivalent. */
    suspend fun lyrics(id: String, title: String, artist: String, durationSec: Int, album: String?): LyricsDto? {
        val cleaned = LyricsUtils.cleanTitleForSearch(title)
        for (name in LyricsProviderRegistry.getDefaultProviderOrder()) {
            val provider = LyricsProviderRegistry.getProviderByName(name)?.takeIf { it.isEnabled(context) } ?: continue
            try {
                provider.getLyrics(context, id, cleaned, artist, durationSec, album).getOrNull()
                    ?.takeIf { it.isNotBlank() }
                    ?.let { return LyricsDto(name, LyricsUtils.filterLyricsCreditLines(it)) }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // next provider
            }
        }
        return null
    }

    private fun YTItem.toItem(): ItemDto? = when (this) {
        is SongItem -> ItemDto("song", id, title, artists.joinToString { it.name }, thumbnail, duration)
        is AlbumItem -> ItemDto("album", browseId, title, listOfNotNull(artists?.joinToString { it.name }, year?.toString()).joinToString(" · "), thumbnail)
        is PlaylistItem -> ItemDto("playlist", id, title, author?.name, thumbnail)
        is ArtistItem -> ItemDto("artist", id, title, null, thumbnail)
        else -> null
    }

    /** The first page is a handful of shelves; Metrolist's home keeps pulling continuations, so do the same (bounded). */
    suspend fun home(maxPages: Int = 4): List<SectionDto> {
        var page = YouTube.home().getOrThrow()
        val sections = page.sections.toMutableList()
        var n = 1
        while (page.continuation != null && n++ < maxPages) {
            page = YouTube.home(continuation = page.continuation).getOrNull() ?: break
            sections += page.sections
        }
        return sections.map { s -> SectionDto(s.title, s.items.mapNotNull { it.toItem() }) }.filter { it.items.isNotEmpty() }
    }

    /** Typed search: song | video | album | artist | playlist. Same ItemDto shape as home/library, so cards render the same. */
    suspend fun searchItems(query: String, type: String): List<ItemDto> {
        val f = when (type) {
            "album" -> YouTube.SearchFilter.FILTER_ALBUM
            "artist" -> YouTube.SearchFilter.FILTER_ARTIST
            "playlist" -> YouTube.SearchFilter.FILTER_COMMUNITY_PLAYLIST
            "video" -> YouTube.SearchFilter.FILTER_VIDEO
            else -> YouTube.SearchFilter.FILTER_SONG
        }
        return YouTube.search(query, f).getOrThrow().items.mapNotNull { it.toItem() }
    }

    suspend fun explore(): List<SectionDto> {
        val e = YouTube.explore().getOrThrow()
        return listOf(
            SectionDto("New releases", e.newReleaseAlbums.mapNotNull { it.toItem() }),
            // Mood/genre browse params are not wired to a page yet, so they are plain labels for now.
            SectionDto("Moods & genres", e.moodAndGenres.map { ItemDto("mood", it.endpoint.params ?: it.endpoint.browseId, it.title) }),
        ).filter { it.items.isNotEmpty() }
    }

    suspend fun history(): List<SectionDto> =
        YouTube.musicHistory().getOrThrow().sections.orEmpty().map { s -> SectionDto(s.title, s.songs.mapNotNull { it.toItem() }) }

    /** kind: songs | albums | artists | playlists, mapped to the same browse ids the Android app uses. */
    suspend fun library(kind: String): List<ItemDto> {
        val id = when (kind) {
            "songs" -> "FEmusic_liked_videos"
            "albums" -> "FEmusic_liked_albums"
            "artists" -> "FEmusic_library_corpus_artists"
            else -> "FEmusic_liked_playlists"
        }
        return YouTube.library(id).completed().getOrThrow().items.mapNotNull { it.toItem() }
    }

    suspend fun album(browseId: String): PageDto {
        val p = YouTube.album(browseId).getOrThrow()
        return PageDto(
            p.album.toItem()!!,
            tracks = p.songs.map { it.toDto() },
            sections = listOf(SectionDto("Other versions", p.otherVersions.mapNotNull { it.toItem() })).filter { it.items.isNotEmpty() },
        )
    }

    suspend fun playlist(id: String): PageDto {
        val p = YouTube.playlist(id).completed().getOrThrow()
        return PageDto(p.playlist.toItem()!!, tracks = p.songs.map { it.toDto() })
    }

    suspend fun artist(id: String): PageDto {
        val p = YouTube.artist(id).getOrThrow()
        return PageDto(
            p.artist.toItem()!!.copy(subtitle = p.subscriberCountText),
            description = p.description,
            sections = p.sections.map { s -> SectionDto(s.title, s.items.mapNotNull { it.toItem() }) }.filter { it.items.isNotEmpty() },
        )
    }

    private fun SongItem.toDto() = TrackDto(id, title, artists.map { it.name }, album?.name, duration, thumbnail, "youtube")
}
