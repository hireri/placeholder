package dev.metrolist.desktop

import com.music.bitchord.data.spotify.SpotifyLibrary
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.add
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray

/**
 * Spotify's library and catalogue through the web player's own queries. It only describes music: a song plays through
 * its YouTube Music match, which the client looks up (`/youtube/match`) before asking YouTube for the stream.
 */
class SpotifySource(override val config: SourceConfig) : MusicSource {
    override val kind = Kind.CATALOG
    override val caps = setOf(Cap.SEARCH, Cap.HOME, Cap.LIBRARY)

    override suspend fun test(): TestResultDto {
        val s = SpotifyAuth.status()
        return when {
            !s.loggedIn -> TestResultDto(false, error = s.error ?: "Sign in to Spotify under Accounts")
            s.error != null -> TestResultDto(false, error = s.error)
            else -> TestResultDto(true, detail = "Signed in as ${s.user?.name}", avatar = s.user?.avatarUrl)
        }
    }

    override suspend fun search(q: String, type: String): List<ItemDto> {
        val (field, parse) = when (type) {
            "song" -> "tracksV2" to { e: JsonElement -> SpotifyParse.track(e.at("item", "data"))?.let(SpotifyParse::song) }
            "album" -> "albumsV2" to { e: JsonElement -> SpotifyParse.album(e.at("data")) }
            "artist" -> "artists" to { e: JsonElement -> SpotifyParse.artist(e.at("data")) }
            "playlist" -> "playlists" to { e: JsonElement -> SpotifyParse.playlist(e.at("data")) }
            else -> return emptyList()
        }
        val data = SpotifyAuth.query("searchDesktop", SEARCH_HASH, buildJsonObject {
            put("searchTerm", q); put("offset", 0); put("limit", 30); put("numberOfTopResults", 5)
            put("includeAudiobooks", false); put("includeArtistHasConcertsField", false); put("includePreReleases", false)
            put("includeLocalConcertsField", false); put("includeAuthors", false)
        })
        return data.at("searchV2", field, "items").list().mapNotNull(parse)
    }

    override suspend fun library(kind: String): List<ItemDto> {
        if (!SpotifyAuth.signedIn) return emptyList()
        return when (kind) {
            "songs" -> tracks(SpotifyLibrary.LIKED_ID).map(SpotifyParse::song)
            "playlists" -> playlists()
            "albums" -> saved("Albums").mapNotNull { SpotifyParse.album(it.at("data")) }
            "artists" -> saved("Artists").mapNotNull { SpotifyParse.artist(it.at("data")) }
            else -> emptyList()
        }
    }

    /** The saved playlists, Spotify's own personal mixes and the new releases feed. One failing leaves the others. */
    override suspend fun home(): List<SectionDto> {
        if (!SpotifyAuth.signedIn) return emptyList()
        val shelves = coroutineScope {
            listOf<suspend () -> SectionDto>(
                { SectionDto("Jump back in", playlists().take(20), "continue") },
                { homeSection(MADE_FOR_YOU, "Made for you") },
                { homeSection(TOP_MIXES, "Your top mixes") },
                { SectionDto("New releases", newReleases(), "discover") },
            ).map { load -> async { runCatching { load() }.onFailure { if (it is CancellationException) throw it } } }.awaitAll()
        }
        val out = shelves.mapNotNull { it.getOrNull() }.filter { it.items.isNotEmpty() }
        if (out.isEmpty()) shelves.firstNotNullOfOrNull { it.exceptionOrNull() }?.let { throw it }
        return out
    }

    private suspend fun homeSection(uri: String, title: String): SectionDto = SpotifyParse.shelf(
        SpotifyAuth.query("homeSection", HOME_SECTION_HASH, buildJsonObject {
            // `sp_t` is the web player's device cookie; Spotify answers without it.
            put("uri", uri); put("timeZone", java.util.TimeZone.getDefault().id); put("sp_t", "")
            put("country", java.util.Locale.getDefault().country.ifEmpty { "US" }); put("sectionItemsLimit", 20); put("sectionItemsOffset", 0)
        }),
        title,
    )

    private suspend fun newReleases(): List<ItemDto> = SpotifyAuth.query("queryWhatsNewFeed", NEW_HASH, buildJsonObject {
        put("offset", 0); put("limit", 30); put("onlyUnPlayedItems", false)
        putJsonArray("includedContentTypes") { add("ALBUM") }; put("includeEpisodeContentRatingsV2", false)
    }).at("whatsNewFeedItems", "items").list().mapNotNull { SpotifyParse.album(it.at("content", "data")) }.distinctBy { it.id }

    override suspend fun album(id: String): PageDto {
        suspend fun page(offset: Int) = SpotifyAuth.query("getAlbum", ALBUM_HASH, buildJsonObject {
            put("uri", "spotify:album:$id"); put("locale", ""); put("offset", offset); put("limit", 50)
        })["albumUnion"]
        val first = page(0)
        val total = first.at("tracksV2", "totalCount").number() ?: 0
        val rows = first.at("tracksV2", "items").list().toMutableList()
        // ponytail: 500 tracks is past any real album; raise the cap if a compilation is cut off.
        while (rows.size < minOf(total, 500)) rows += page(rows.size).at("tracksV2", "items").list().ifEmpty { break }
        return SpotifyParse.albumPage(id, first, rows)
    }

    override suspend fun artist(id: String): PageDto {
        val overview = SpotifyAuth.query("queryArtistOverview", ARTIST_HASH, buildJsonObject {
            put("uri", "spotify:artist:$id"); put("locale", ""); put("includePrerelease", false)
        })["artistUnion"]
        // The overview stops at ten releases of each kind. ponytail: the newest 50; page further if a long catalogue is cut off.
        val truncated = listOf("albums", "singles", "compilations").any {
            (overview.at("discography", it, "totalCount").number() ?: 0) > overview.at("discography", it, "items").list().size
        }
        val all = if (!truncated) null else runCatching {
            SpotifyAuth.query("queryArtistDiscographyAll", DISCOGRAPHY_HASH, buildJsonObject {
                put("uri", "spotify:artist:$id"); put("offset", 0); put("limit", 50); put("order", "DATE_DESC")
            })["artistUnion"].at("discography", "all", "items")
        }.onFailure { if (it is CancellationException) throw it }.getOrNull()
        return SpotifyParse.artistPage(id, overview, all)
    }

    override suspend fun playlist(id: String): PageDto {
        val item = if (id == SpotifyLibrary.LIKED_ID) {
            LIKED
        } else {
            val header = SpotifyAuth.query("fetchPlaylist", PLAYLIST_HASH, buildJsonObject {
                put("uri", "spotify:playlist:$id"); put("offset", 0); put("limit", 1); put("enableWatchFeedEntrypoint", false)
            })["playlistV2"]
            val owner = header.at("ownerV2", "data")
            SpotifyParse.playlist(header, id)?.let { it.copy(author = it.subtitle, authorAvatar = SpotifyParse.image(owner.at("avatar", "sources"))) }
                ?: ItemDto("playlist", id, "Spotify playlist")
        }
        return PageDto(item, tracks = tracks(id))
    }

    override suspend fun stream(id: String): StreamDto = throw UnsupportedOperationException("Spotify songs play through their YouTube Music match")

    companion object {
        private val LIKED by lazy { ItemDto("playlist", SpotifyLibrary.LIKED_ID, "Liked Songs", thumbnail = LIKED_COVER) }

        /** Liked Songs, then every playlist. Not BitChord's reader, which fetches page after page with no pause. */
        suspend fun playlists(): List<ItemDto> = listOf(LIKED) + saved("Playlists").mapNotNull { row ->
            SpotifyParse.playlist(row.at("data"), (row.at("_uri") as? JsonPrimitive)?.contentOrNull?.takeIf { it.startsWith("spotify:playlist:") }?.substringAfterLast(':') ?: return@mapNotNull null)
        }

        private suspend fun saved(filter: String): List<JsonElement?> {
            val out = mutableListOf<JsonElement?>()
            var offset = 0
            while (offset < 1_000) {
                val page = SpotifyAuth.query("libraryV3", LIBRARY_HASH, buildJsonObject {
                    putJsonArray("filters") { add(filter) }
                    put("order", null as String?); put("textFilter", ""); putJsonArray("features") {}
                    put("limit", 50); put("offset", offset); put("flatten", true); putJsonArray("expandedFolders") {}
                    put("folderUri", null as String?); put("includeFoldersWhenFlattening", false)
                }).at("me", "libraryV3")
                val rows = page.at("items").list()
                out += rows.map { it.at("item") }
                offset += rows.size
                if (rows.isEmpty() || offset >= (page.at("totalCount").number() ?: 0)) break
            }
            return out
        }

        /** Spotify's own Liked Songs artwork; the collection has no cover of its own. */
        private const val LIKED_COVER = "https://misc.scdn.co/liked-songs/liked-songs-640.png"
        private const val SEARCH_HASH = "eef7cc54888d91bdd6802623477873caa3948ae173a0c34fd86827b267e94c03"
        private const val ALBUM_HASH = "0b6ce3511e1b02e4a40fa95440f992acbe85080eb308af9194322d708fc1914f"
        private const val ARTIST_HASH = "4bc52527bb77a5f8bbb9afe491e9aa725698d29ab73bff58d49169ee29800167"
        private const val DISCOGRAPHY_HASH = "5e07d323febb57b4a56a42abbf781490e58764aa45feb6e3dc0591564fc56599"
        private const val HOME_SECTION_HASH = "eb3fba2d388cf4fc4d696b1757a58584e9538a3b515ea742e9cc9465807340be"
        private const val NEW_HASH = "d889c8c936ab192af8ced595427f5ba2acdf63478fdc0a181c8d477f8322630e"
        private const val MADE_FOR_YOU = "spotify:section:0JQ5DAUnp4wcj0bCb3wh3S"
        private const val TOP_MIXES = "spotify:section:0JQ5DAnM3wGh0gz1MXnu89"
        /** Spotify's DJ is not a playlist anything but its own apps can play. */
        const val DJ_ID = "37i9dQZF1EYkqdzj48dyYq"
        private const val LIBRARY_HASH = "973e511ca44261fda7eebac8b653155e7caee3675abb4fb110cc1b8c78b091c3"
        private const val PLAYLIST_HASH = "346811f856fb0b7e4f6c59f8ebea78dd081c6e2fb01b77c954b26259d5fc6763"

        private const val LIKED_HASH = "087278b20b743578a6262c2b0b4bcd20d879c503cc359a2285baf083ef944240"

        /**
         * A playlist's songs (or Liked Songs). Not BitChord's reader: it looks for `duration` where playlists now send
         * `trackDuration`, and drops the artist and album ids.
         */
        suspend fun tracks(playlistId: String): List<TrackDto> {
            val liked = playlistId == SpotifyLibrary.LIKED_ID
            val out = mutableListOf<TrackDto>()
            var offset = 0
            // ponytail: 10,000 songs, a hundred a second; Spotify's own playlist limit.
            while (offset < 10_000) {
                val data = if (liked) {
                    SpotifyAuth.query("fetchLibraryTracks", LIKED_HASH, buildJsonObject { put("offset", offset); put("limit", 100) })
                } else {
                    SpotifyAuth.query("fetchPlaylist", PLAYLIST_HASH, buildJsonObject {
                        put("uri", "spotify:playlist:$playlistId"); put("offset", offset); put("limit", 100); put("enableWatchFeedEntrypoint", false)
                    })
                }
                val page = if (liked) data.at("me", "library", "tracks") else data.at("playlistV2", "content")
                val rows = (page.at("items") as? JsonArray).orEmpty()
                out += rows.mapNotNull { SpotifyParse.row(it) }
                offset += rows.size
                val total = (page.at("totalCount") as? JsonPrimitive)?.longOrNull ?: 0
                if (rows.isEmpty() || offset >= total) break
            }
            return out
        }
    }
}

internal fun JsonElement?.at(vararg path: String): JsonElement? = path.fold(this) { e, k -> (e as? JsonObject)?.get(k) }
private fun JsonElement?.list(): List<JsonElement> = (this as? JsonArray) ?: emptyList()
private fun JsonElement?.text(): String? = (this as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotEmpty() }
private fun JsonElement?.number(): Long? = (this as? JsonPrimitive)?.longOrNull

/** Pathfinder replies to the daemon's shapes. Anything without an id and a name (removed or unreleased items) is skipped. */
internal object SpotifyParse {
    private fun id(e: JsonElement?, kind: String) = e.at("uri").text()?.takeIf { it.startsWith("spotify:$kind:") }?.substringAfterLast(':')
    /** The largest picture; sources without sizes keep their order. */
    fun image(sources: JsonElement?) = sources.list().maxByOrNull { (it.at("width") ?: it.at("maxWidth")).number() ?: 0 }.at("url").text()
    private fun names(artists: JsonElement?) = artists.at("items").list().mapNotNull { it.at("profile", "name").text() }
    private fun year(e: JsonElement?) = e.at("date", "year").number()?.toString() ?: e.at("date", "isoString").text()?.take(4)
    private fun line(vararg parts: String?) = parts.filterNotNull().filter { it.isNotEmpty() }.joinToString(" · ").ifEmpty { null }

    /** A playlist row (`itemV2.data`) or a Liked Songs row (`track.data`, with the uri beside it). */
    fun row(e: JsonElement): TrackDto? =
        track(e.at("itemV2", "data") ?: e.at("track", "data"), uri = e.at("track", "_uri").text())

    /** `album` and `cover` stand in where the row does not repeat them (an album's own track list). */
    fun track(e: JsonElement?, album: ItemDto? = null, uri: String? = null): TrackDto? {
        val id = id(e, "track") ?: uri?.takeIf { it.startsWith("spotify:track:") }?.substringAfterLast(':') ?: return null
        val of = e.at("albumOfTrack")
        return TrackDto(
            id = id, title = e.at("name").text() ?: return null, artists = names(e.at("artists")),
            album = of.at("name").text() ?: album?.title,
            durationSec = (e.at("trackDuration") ?: e.at("duration")).at("totalMilliseconds").number()?.div(1000)?.toInt(),
            thumbnail = image(of.at("coverArt", "sources")) ?: album?.thumbnail,
            source = "spotify",
            artistId = id(e.at("artists", "items").list().firstOrNull(), "artist"),
            albumId = id(of, "album") ?: album?.id,
            explicit = e.at("contentRating", "label").text() == "EXPLICIT",
        )
    }

    fun song(t: TrackDto) = ItemDto("song", t.id, t.title, t.artists.joinToString(", ").ifEmpty { null }, t.thumbnail, t.durationSec, t.artistId, t.album, t.albumId, t.explicit)

    fun album(e: JsonElement?): ItemDto? {
        val id = id(e, "album") ?: return null
        return ItemDto(
            "album", id, e.at("name").text() ?: return null, line(names(e.at("artists")).joinToString(", "), year(e)),
            image(e.at("coverArt", "sources")), artistId = id(e.at("artists", "items").list().firstOrNull(), "artist"),
        )
    }

    fun artist(e: JsonElement?): ItemDto? {
        val id = id(e, "artist") ?: return null
        return ItemDto("artist", id, e.at("profile", "name").text() ?: return null, thumbnail = image(e.at("visuals", "avatarImage", "sources")))
    }

    fun playlist(e: JsonElement?, knownId: String? = null): ItemDto? {
        val id = knownId ?: id(e, "playlist") ?: return null
        return ItemDto(
            "playlist", id, e.at("name").text() ?: return null, e.at("ownerV2", "data", "name").text(),
            image(e.at("images", "items").list().firstOrNull().at("sources")),
        )
    }

    fun albumPage(id: String, e: JsonElement?, rows: List<JsonElement>): PageDto {
        val artist = e.at("artists", "items").list().firstOrNull()
        val item = ItemDto(
            "album", id, e.at("name").text() ?: "Album", line(names(e.at("artists")).joinToString(", "), year(e)),
            image(e.at("coverArt", "sources")), artistId = id(artist, "artist"),
        )
        return PageDto(item, tracks = rows.mapNotNull { track(it.at("track"), item) })
    }

    /** One of Home's personal sections: its playlists, under Spotify's own title. */
    fun shelf(data: JsonElement?, fallback: String): SectionDto {
        val sections = data.at("homeSections", "sections").list()
        val items = sections.flatMap { it.at("sectionItems", "items").list() }.mapNotNull { row ->
            val d = row.at("content", "data")
            playlist(d)?.takeIf { it.id != SpotifySource.DJ_ID }?.let { p -> p.copy(subtitle = plain(d.at("description").text()) ?: p.subtitle) }
        }.distinctBy { it.id }
        return SectionDto(sections.firstOrNull().at("data", "title", "text").text() ?: fallback, items, "mixes")
    }

    /** Descriptions and biographies carry links and HTML entities. */
    private fun plain(s: String?) = s?.replace(Regex("<[^>]*>"), "")?.replace("&#x27;", "'")?.replace("&#39;", "'")
        ?.replace("&#x2F;", "/")?.replace("&quot;", "\"")?.replace("&lt;", "<")?.replace("&gt;", ">")?.replace("&amp;", "&")?.trim()?.ifEmpty { null }

    /** `all` is the full discography's rows, when the overview's lists were cut short. */
    fun artistPage(id: String, e: JsonElement?, all: JsonElement? = null): PageDto {
        val name = e.at("profile", "name").text() ?: "Artist"
        val item = ItemDto(
            "artist", id, name, thumbnail = image(e.at("visuals", "avatarImage", "sources")),
            banner = image(e.at("visuals", "headerImage", "sources")),
        )
        fun release(r: JsonElement?, own: Boolean = true): ItemDto? {
            val albumId = id(r, "album") ?: return null
            val by = if (own) name else names(r.at("artists")).joinToString(", ")
            return ItemDto(
                "album", albumId, r.at("name").text() ?: return null, line(by, year(r)), image(r.at("coverArt", "sources")),
                artistId = if (own) id else id(r.at("artists", "items").list().firstOrNull(), "artist"),
            )
        }
        /** Discography lists group a release's editions; the first stands for them. */
        fun first(group: JsonElement) = group.at("releases", "items").list().firstOrNull()
        val rest = all.list().mapNotNull(::first)
        fun own(key: String, vararg types: String) =
            (e.at("discography", key, "items").list().mapNotNull(::first) + rest.filter { it.at("type").text() in types })
                .mapNotNull { release(it) }.distinctBy { it.id }
                // Newest first once the two lists are joined.
                .let { list -> if (rest.isEmpty()) list else list.sortedByDescending { it.subtitle?.substringAfterLast(" · ") } }
        fun section(title: String, items: List<ItemDto>) = if (items.isEmpty()) null else SectionDto(title, items)
        val top = e.at("discography", "topTracks", "items").list().mapNotNull { track(it.at("track")) }.map(::song)
        return PageDto(
            item,
            description = plain(e.at("profile", "biography", "text").text()),
            // Top songs are a section, not `tracks`: a page with tracks is drawn as one list and its shelves are dropped.
            sections = listOfNotNull(
                section("Top songs", top),
                section("Popular releases", e.at("discography", "popularReleasesAlbums", "items").list().mapNotNull { release(it) }),
                section("Albums", own("albums", "ALBUM")),
                section("Singles and EPs", own("singles", "SINGLE", "EP")),
                section("Compilations", own("compilations", "COMPILATION")),
                section("Featuring $name", e.at("relatedContent", "featuringV2", "items").list().mapNotNull { playlist(it.at("data")) }),
                section("Artist playlists", e.at("profile", "playlistsV2", "items").list().mapNotNull { playlist(it.at("data")) }),
                section("Appears on", e.at("relatedContent", "appearsOn", "items").list().mapNotNull { release(first(it), own = false) }),
                section("Fans also like", e.at("relatedContent", "relatedArtists", "items").list().mapNotNull(::artist)),
            ),
            monthlyListeners = e.at("stats", "monthlyListeners").number()?.let { "%,d monthly listeners".format(it) },
        )
    }
}
