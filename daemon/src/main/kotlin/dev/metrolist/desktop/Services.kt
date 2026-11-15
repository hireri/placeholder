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
import com.metrolist.innertubex.extraction.AudioQuality as InnerTubeXAudioQuality
import com.metrolist.innertubex.extraction.ContentHints
import com.metrolist.innertubex.extraction.InnerTubeExtractor
import com.metrolist.innertubex.extraction.generateClientPlaybackNonce
import com.metrolist.innertubex.sabr.SabrBootstrap
import com.metrolist.innertubex.sabr.SabrVideoStream
import kotlinx.coroutines.flow.Flow
import java.util.concurrent.ConcurrentHashMap
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
            TrackDto(it.id, it.title, it.artist.split(", ").filter(String::isNotBlank), it.album, it.durationMs.takeIf { it > 0 }?.div(1000), it.imageUrl, "spotify")
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
            durationSec = data.videoDetails?.lengthSeconds?.toIntOrNull()?.takeIf { it > 0 }
                ?: data.format.approxDurationMs?.toLongOrNull()?.div(1000)?.toInt(),
        )
    }

    private val videoBootstraps = ConcurrentHashMap<String, Pair<Long, SabrBootstrap>>()

    /**
     * SABR session for a song's video track, cached ten minutes so seeks don't re-extract. YouTube serves video only over SABR here.
     * Upstream's `InnerTubeXPlayer` keeps its extractor private and drops video, so it is read by reflection after `prewarm()` builds it.
     * ponytail: reflection on upstream internals; breaks loudly if they are renamed, swap for an upstream API if one appears.
     */
    suspend fun videoBootstrap(videoId: String, maxHeight: Int): SabrBootstrap {
        val key = "$videoId/$maxHeight"
        videoBootstraps[key]?.takeIf { System.currentTimeMillis() - it.first < 10 * 60_000 }?.let { return it.second }
        InnerTubeXPlayer.prewarm()
        val bundle = InnerTubeXPlayer::class.java.getDeclaredField("currentBundle").apply { isAccessible = true }.get(InnerTubeXPlayer)
            ?: error("extractor not ready")
        val extractor = bundle.javaClass.getDeclaredField("extractor").apply { isAccessible = true }.get(bundle) as InnerTubeExtractor
        val hints = ContentHints(wantVideo = true, maxVideoHeight = maxHeight).withStreamCapabilities(allowHls = false, allowSabr = true, allowBoundedRange = true)
        val s = extractor.extract(videoId, hints, emptySet(), InnerTubeXAudioQuality.LOW, generateClientPlaybackNonce()) ?: error("no stream for $videoId")
        val b = s.sabrVideoBootstrap ?: error("no video stream for $videoId")
        // The desktop client decodes with stock GStreamer: prefer H.264, then VP9 (AV1 decoders are rarely installed), tallest within the cap.
        val pick = b.videoVariants.filter { it.height <= maxHeight && !it.mimeType.contains("av01") }
            .maxWithOrNull(compareBy({ it.mimeType.contains("avc1") }, { it.height }))
        val chosen = pick?.let {
            b.copy(selectedVideoFormat = it.format, selectedVideoWidth = it.width, selectedVideoHeight = it.height,
                selectedVideoMimeType = it.mimeType, selectedVideoContentLengthBytes = it.contentLengthBytes, selectedVideoBitrate = it.bitrate)
        } ?: b
        videoBootstraps[key] = System.currentTimeMillis() to chosen
        return chosen
    }

    /** The video track as one progressive stream from `startMs`. The HTTP response's backpressure paces it to the client. */
    fun videoBytes(b: SabrBootstrap, startMs: Long): Flow<ByteArray> {
        val t0 = System.currentTimeMillis()
        return SabrVideoStream(
            httpClient = YouTube.extractionTransport().httpClient,
            bootstrap = b,
            playbackPositionMs = { startMs + System.currentTimeMillis() - t0 },
            initialPlayerTimeMs = startMs,
        ).bytes()
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
        is SongItem -> ItemDto("song", id, title, artists.joinToString { it.name }, thumbnail, duration, artistId = artists.firstOrNull()?.id)
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

    /** Likes or unlikes a song on YouTube Music. */
    suspend fun like(videoId: String, on: Boolean) {
        YouTube.likeVideo(videoId, on).getOrThrow()
    }

    /** Saves or removes an album or playlist. Albums are saved through their playlist id, which the album page carries. */
    suspend fun save(kind: String, id: String, on: Boolean) {
        val playlistId = if (kind == "albums") YouTube.album(id).getOrThrow().album.playlistId else id
        YouTube.likePlaylist(playlistId, on).getOrThrow()
    }

    /** Subscribes or unsubscribes a channel (artist). */
    suspend fun subscribe(channelId: String, on: Boolean) {
        YouTube.subscribeChannel(channelId, on).getOrThrow()
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
            channelId = p.artist.channelId,
            subscribed = p.isSubscribed,
            monthlyListeners = p.monthlyListenerCount,
            sections = p.sections.map { s -> SectionDto(s.title, s.items.mapNotNull { it.toItem() }) }.filter { it.items.isNotEmpty() },
        )
    }

    private fun SongItem.toDto() = TrackDto(id, title, artists.map { it.name }, album?.name, duration, thumbnail, "youtube", artists.firstOrNull()?.id, album?.id)
}
