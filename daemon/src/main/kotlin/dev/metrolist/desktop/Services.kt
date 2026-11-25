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
import com.metrolist.innertube.models.WatchEndpoint
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
import java.text.Normalizer
import java.util.Locale
import java.util.concurrent.ConcurrentHashMap
import com.music.bitchord.data.spotify.SpotifyLibrary
import kotlinx.coroutines.CancellationException
import kotlin.math.abs

object Services {
    /** Upstream code wants a Context; on desktop it is the shim. */
    val context = Context()

    private val matchGate = RateGate(permits = 2, spacingMs = 350)
    private val writeGate = RateGate(permits = 1, spacingMs = 200, backoffBaseMs = 2_000)
    private val matchCache = MatchCache()

    init {
        InnerTubeXPlayer.initialize(context)
    }

    suspend fun search(query: String): List<TrackDto> =
        YouTube.search(query, YouTube.SearchFilter.FILTER_SONG).getOrThrow().items.filterIsInstance<SongItem>().map { it.toDto() }

    /**
     * The YouTube Music song that is the Spotify track, or null if none is close enough. The first search hit is
     * not trusted: title, version markers, artists and duration decide, and a different-artist row only wins
     * when its runtime is identical (a cover can share a title and a length by coincidence, rarely both closely).
     */
    suspend fun matchTrack(id: String?, title: String, artist: String, durationSec: Int?, album: String?): Result<TrackDto?> {
        val key = matchKey(id, title, artist, durationSec, album)
        matchCache.get(key)?.let { return Result.success(it.track) }

        val primary = artist.split(",").firstOrNull()?.trim().orEmpty()
        val queries = listOf(listOf(title, primary).filter { it.isNotBlank() }.joinToString(" "), title.trim())
            .filter { it.isNotBlank() }.distinct()
        var failed = false
        for (query in queries) {
            val result = matchGate.run {
                YouTube.search(query, YouTube.SearchFilter.FILTER_SONG).map { p -> p.items.filterIsInstance<SongItem>() }
            }
            if (result.isFailure) {
                failed = true
                break
            }
            val hits = result.getOrDefault(emptyList())
            val best = hits.mapNotNull { s -> matchScore(s, title, artist, durationSec, album)?.let { s to it } }.maxByOrNull { it.second }
            if (best != null) {
                val track = best.first.toDto()
                matchCache.put(key, track)
                return Result.success(track)
            }
        }
        if (failed) return Result.failure(IllegalStateException("YouTube search failed"))
        matchCache.put(key, null)
        return Result.success(null)
    }

    private fun matchKey(id: String?, title: String, artist: String, durationSec: Int?, album: String?): String {
        if (!id.isNullOrBlank()) return "spotify:$id"
        return "meta:${normalized(title)}|${normalized(artist)}|${durationSec ?: ""}|${normalized(album.orEmpty())}"
    }

    private suspend fun createEmptyPlaylist(title: String): String = writeGate.run {
        runCatching { YouTube.createPlaylist(title) }
    }.getOrElse { throw IllegalStateException("Could not create the playlist: ${it.message ?: "YouTube refused"}") }
        .also { if (it.isBlank()) throw IllegalStateException("YouTube did not return a playlist id") }

    /** Creates a private playlist and seeds it in as few requests as possible, falling back to paced single adds. */
    suspend fun createPlaylist(title: String, videoIds: List<String>): CreatedPlaylistDto {
        val ids = videoIds.distinct()
        val stored = CredentialStore().load().youtube
        if (stored != null) {
            runCatching { YouTubeWrite.createPlaylist(title, ids, stored.cookie, stored.visitorData, stored.dataSyncId) }
                .getOrNull()?.let { return CreatedPlaylistDto(id = it, added = ids.size, total = ids.size) }
        }
        val id = createEmptyPlaylist(title)
        var added = 0
        if (ids.isNotEmpty() && stored != null) {
            added = runCatching { YouTubeWrite.addAll(id, ids, stored.cookie, stored.visitorData, stored.dataSyncId) }.getOrNull() ?: 0
        }
        if (added == 0 && ids.isNotEmpty()) {
            for (videoId in ids) {
                val ok = writeGate.run { YouTube.addToPlaylist(id, videoId) }.isSuccess
                if (ok) added++
            }
        }
        return CreatedPlaylistDto(id = id, added = added, total = ids.size)
    }

    private val versionWords = listOf("remix", "live", "acoustic", "instrumental", "unplugged", "extended", "sped up", "slowed", "nightcore", "karaoke", "cover")

    private fun normalized(s: String) = Normalizer.normalize(s, Normalizer.Form.NFKD)
        .replace(Regex("\\p{M}+"), "")
        .lowercase(Locale.ROOT)
        .replace(Regex("[^a-z0-9]+"), " ")
        .trim()

    private fun titleCore(raw: String, artist: String): String {
        var s = raw
        repeat(3) { s = s.replace(Regex("""[\(\[\{][^\)\]\}]*[\)\]\}]"""), " ") }
        val nArtist = normalized(artist)
        val cut = s.split(" - ", limit = 2)
        if (cut.size == 2) {
            val head = normalized(cut[0])
            s = if (nArtist.isNotEmpty() && (head == nArtist || nArtist.contains(head) || head.contains(nArtist))) cut[1] else cut[0]
        }
        return normalized(s)
    }

    private fun versions(s: String): Set<String> {
        val n = normalized(s)
        return versionWords.filter { Regex("""\b${Regex.escape(it)}\b""").containsMatchIn(n) }.toSet()
    }

    private fun artistNames(raw: String): Set<String> =
        raw.split(Regex("""(?i)\s*(?:,|&|/|\bfeat\.?\b|\bft\.?\b|\bfeaturing\b|\sx\s)\s*"""))
            .map(::normalized)
            .filter { it.isNotEmpty() }
            .toSet()

    private fun matchScore(hit: SongItem, title: String, artist: String, durationSec: Int?, album: String?): Int? {
        val want = titleCore(title, artist)
        val got = titleCore(hit.title, artist)
        if (want.isBlank() || got.isBlank()) return null
        val exact = want == got
        val contained = want.length > 3 && got.length > 3 && (want.contains(got) || got.contains(want))
        if (!exact && !contained) return null
        if (versions(title) != versions(hit.title)) return null
        val shared = artistNames(artist).any { a -> artistNames(hit.artists.joinToString { it.name }).any { it == a } }
        val hitDuration = hit.duration
        val drift = if (durationSec != null && hitDuration != null) abs(durationSec - hitDuration) else null
        if (drift != null && drift > 30) return null
        if (!shared && !(exact && drift != null && drift <= 2)) return null
        var score = if (exact) 100 else 60
        if (shared) score += 40
        score += when {
            drift == null -> 0
            drift <= 3 -> 25
            drift <= 10 -> 15
            else -> 5
        }
        val hitAlbum = hit.album?.name
        if (!album.isNullOrBlank() && hitAlbum != null && normalized(hitAlbum) == normalized(album)) score += 10
        return score
    }

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
        val data = InnerTubeXPlayer.playerResponseForPlayback(videoId, null, Settings.audioQuality(), ConnectivityManager()).getOrThrow()
        return StreamDto(
            url = data.streamUrl,
            headers = data.streamHeaders,
            mimeType = data.format.mimeType,
            expiresInSeconds = data.streamExpiresInSeconds,
            client = data.streamClient,
            requireBoundedRange = data.requireBoundedRange,
            rangeChunkSizeBytes = data.rangeChunkSizeBytes,
            useRangeChunks = data.useRangeChunks,
            bitrate = data.format.bitrate,
            durationSec = data.videoDetails?.lengthSeconds?.toIntOrNull()?.takeIf { it > 0 }
                ?: data.format.approxDurationMs?.toLongOrNull()?.div(1000)?.toInt(),
        )
    }

    private val videoBootstraps = ConcurrentHashMap<String, Pair<Long, SabrBootstrap>>()

    /**
     * SABR session for a song's video track, cached ten minutes per song so seeks and quality changes don't re-extract (~3-10 s).
     * YouTube serves video only over SABR here.
     * Upstream's `InnerTubeXPlayer` keeps its extractor private and drops video, so it is read by reflection after `prewarm()` builds it.
     * ponytail: reflection on upstream internals; breaks loudly if they are renamed, swap for an upstream API if one appears.
     */
    suspend fun videoBootstrap(videoId: String, maxHeight: Int): SabrBootstrap {
        val b = videoBootstraps[videoId]?.takeIf { System.currentTimeMillis() - it.first < 10 * 60_000 }?.second
            ?: extractVideo(videoId).also { videoBootstraps[videoId] = System.currentTimeMillis() to it }
        // The desktop client decodes with stock ffmpeg: prefer H.264, then VP9 (AV1 decoders are rarely installed), tallest within the cap.
        val pick = b.videoVariants.filter { it.height <= maxHeight && !it.mimeType.contains("av01") }
            .maxWithOrNull(compareBy({ it.mimeType.contains("avc1") }, { it.height }))
        return pick?.let {
            b.copy(selectedVideoFormat = it.format, selectedVideoWidth = it.width, selectedVideoHeight = it.height,
                selectedVideoMimeType = it.mimeType, selectedVideoContentLengthBytes = it.contentLengthBytes, selectedVideoBitrate = it.bitrate)
        } ?: b
    }

    private suspend fun extractVideo(videoId: String): SabrBootstrap {
        InnerTubeXPlayer.prewarm()
        val bundle = InnerTubeXPlayer::class.java.getDeclaredField("currentBundle").apply { isAccessible = true }.get(InnerTubeXPlayer)
            ?: error("extractor not ready")
        val extractor = bundle.javaClass.getDeclaredField("extractor").apply { isAccessible = true }.get(bundle) as InnerTubeExtractor
        // The client's tallest choice, so every quality it offers is among the variants.
        val hints = ContentHints(wantVideo = true, maxVideoHeight = 1080).withStreamCapabilities(allowHls = false, allowSabr = true, allowBoundedRange = true)
        val s = extractor.extract(videoId, hints, emptySet(), InnerTubeXAudioQuality.LOW, generateClientPlaybackNonce()) ?: error("no stream for $videoId")
        return s.sabrVideoBootstrap ?: error("no video stream for $videoId")
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

    /** First provider (in the stored order) that has lyrics. Upstream's LyricsHelper is DB-bound, so this is the thin desktop equivalent. */
    suspend fun lyrics(id: String, title: String, artist: String, durationSec: Int, album: String?): LyricsDto? {
        val cleaned = LyricsUtils.cleanTitleForSearch(title)
        for (name in Settings.lyricsOrder()) {
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
        is SongItem -> ItemDto("song", id, title, artists.joinToString { it.name }, thumbnail, duration, artistId = artists.firstOrNull()?.id, album = album?.name, albumId = album?.id)
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

    suspend fun suggest(query: String): SuggestDto {
        val s = YouTube.searchSuggestions(query).getOrThrow()
        return SuggestDto(s.queries, s.recommendedItems.mapNotNull { it.toItem() })
    }

    suspend fun explore(): List<SectionDto> {
        val e = YouTube.explore().getOrThrow()
        return listOf(
            SectionDto("New releases", e.newReleaseAlbums.mapNotNull { it.toItem() }),
            // Mood/genre browse params are not wired to a page yet, so they are plain labels for now.
            SectionDto("Moods & genres", e.moodAndGenres.map { ItemDto("mood", it.endpoint.params ?: it.endpoint.browseId, it.title) }),
        ).filter { it.items.isNotEmpty() }
    }

    /** One song with its album (feed items often lack it). */
    suspend fun song(videoId: String): TrackDto =
        YouTube.queue(listOf(videoId)).getOrThrow().firstOrNull()?.toDto() ?: throw NoSuchElementException("song not found")

    /** What YouTube Music would play after a song (its "RDAMVM" radio mix), without the seed itself: feeds endless mode. */
    suspend fun radio(videoId: String): List<TrackDto> =
        YouTube.next(WatchEndpoint(videoId = videoId, playlistId = "RDAMVM$videoId")).getOrThrow().items.filter { it.id != videoId }.map { it.toDto() }

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
