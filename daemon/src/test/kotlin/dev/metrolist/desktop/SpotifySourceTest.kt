package dev.metrolist.desktop

import kotlinx.serialization.json.Json
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

/** Fixtures follow the field layout of live pathfinder replies, cut to what is read. */
class SpotifySourceTest {
    private fun j(s: String) = Json.parseToJsonElement(s)

    private val track = """{"uri":"spotify:track:t1","name":"One","duration":{"totalMilliseconds":201500},"contentRating":{"label":"EXPLICIT"},
        "artists":{"items":[{"uri":"spotify:artist:a1","profile":{"name":"A"}},{"uri":"spotify:artist:a2","profile":{"name":"B, C"}}]},
        "albumOfTrack":{"uri":"spotify:album:al1","name":"Disc","coverArt":{"sources":[{"url":"small","width":64},{"url":"big","width":640}]}}}"""

    @Test
    fun trackKeepsItsSpotifyIdentityAndNamespacesOnce() {
        val t = SpotifyParse.track(j(track))!!
        assertEquals(listOf("A", "B, C"), t.artists)
        assertEquals(201, t.durationSec)
        assertEquals("big", t.thumbnail)
        assertEquals(true, t.explicit)
        val out = t.ns("spotify")
        assertEquals("spotify:t1", out.id)
        assertEquals("spotify:a1", out.artistId)
        assertEquals("spotify:al1", out.albumId)
        assertEquals("spotify", out.source)
    }

    @Test
    fun albumTracksTakeCoverAndAlbumFromThePage() {
        val head = j("""{"name":"Disc","date":{"isoString":"2013-05-17T00:00:00Z"},"artists":{"items":[{"uri":"spotify:artist:a1","profile":{"name":"A"}}]},
            "coverArt":{"sources":[{"url":"cover","width":300}]}}""")
        val page = SpotifyParse.albumPage("al1", head, listOf(j("""{"track":{"uri":"spotify:track:t2","name":"Two"}}"""), j("""{"track":{}}""")))
        assertEquals("A · 2013", page.item.subtitle)
        assertEquals(1, page.tracks.size)
        assertEquals("cover", page.tracks[0].thumbnail)
        assertEquals("al1", page.tracks[0].albumId)
    }

    @Test
    fun artistPageHasTopTracksAndReleases() {
        val page = SpotifyParse.artistPage("a1", j("""{"profile":{"name":"A"},"discography":{"topTracks":{"items":[{"track":$track}]},
            "albums":{"items":[{"releases":{"items":[{"uri":"spotify:album:al1","name":"Disc","date":{"year":2013}}]}}]},"singles":{"items":[]}}}"""))
        // Top songs must stay a section: with `tracks` set the client draws only a track list.
        assertEquals(0, page.tracks.size)
        assertEquals(listOf("Top songs", "Albums"), page.sections.map { it.title })
        assertEquals("a1", page.sections[1].items[0].artistId)
    }

    @Test
    fun fullDiscographyFillsTheShelvesWithoutRepeats() {
        fun group(id: String, year: Int, type: String) = """{"releases":{"items":[{"uri":"spotify:album:$id","name":"$id","type":"$type","date":{"year":$year}}]}}"""
        val page = SpotifyParse.artistPage(
            "a1", j("""{"profile":{"name":"A"},"discography":{"albums":{"items":[${group("old", 2001, "ALBUM")}]}}}"""),
            j("[${group("new", 2020, "ALBUM")},${group("old", 2001, "ALBUM")},${group("ep", 2010, "EP")}]"),
        )
        assertEquals(listOf("new", "old"), page.sections.first { it.title == "Albums" }.items.map { it.id })
        assertEquals(listOf("ep"), page.sections.first { it.title == "Singles and EPs" }.items.map { it.id })
    }

    @Test
    fun homeShelfKeepsSpotifysTitleAndSkipsTheDj() {
        fun card(id: String) = """{"content":{"data":{"uri":"spotify:playlist:$id","name":"$id","description":"<a href=x>A</a> &amp; B"}}}"""
        val shelf = SpotifyParse.shelf(
            j("""{"homeSections":{"sections":[{"data":{"title":{"text":"Made For you"}},"sectionItems":{"items":[${card("mix")},${card(SpotifySource.DJ_ID)},{"content":null}]}}]}}"""),
            "Made for you",
        )
        assertEquals("Made For you", shelf.title)
        assertEquals(listOf("mix" to "A & B"), shelf.items.map { it.id to it.subtitle })
    }

    @Test
    fun playlistAndLikedRowsKeepTheirDuration() {
        val inPlaylist = SpotifyParse.row(j("""{"itemV2":{"data":{"uri":"spotify:track:t1","name":"One","trackDuration":{"totalMilliseconds":61000}}}}"""))!!
        assertEquals("t1" to 61, inPlaylist.id to inPlaylist.durationSec)
        val liked = SpotifyParse.row(j("""{"track":{"_uri":"spotify:track:t2","data":{"name":"Two","duration":{"totalMilliseconds":62000}}}}"""))!!
        assertEquals("t2" to 62, liked.id to liked.durationSec)
        assertNull(SpotifyParse.row(j("""{"itemV2":{"data":{"uri":"spotify:local:x","name":"Mine"}}}""")))
    }

    @Test
    fun unavailableRowsAreSkipped() {
        assertNull(SpotifyParse.album(j("""{"__typename":"NotFound"}""")))
        assertNull(SpotifyParse.artist(null))
        assertNull(SpotifyParse.track(j("""{"uri":"spotify:episode:e1","name":"Pod"}""")))
    }

    @Test
    fun spotifyIsABuiltInSourceThatNamespacesItsIds() {
        assert("spotify" in Sources.types && Sources.isSingleton("spotify"))
        assertEquals("spotify:abc", Sources.fullId("spotify", "abc"))
    }
}
