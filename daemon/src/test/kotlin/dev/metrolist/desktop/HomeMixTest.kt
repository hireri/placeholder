package dev.metrolist.desktop

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class HomeMixTest {
    @Test
    fun youtubeTitlesMapToShelves() {
        val cases = mapOf("Listen again" to "continue", "Your forgotten favorites" to "played", "New releases" to "recent", "Supermix" to "mixes",
            "Picks for you" to "mixes", "Trending" to "discover", "Top charts" to "discover", "Quick picks" to "mixes", "Jazz classics" to null)
        cases.forEach { (t, shelf) -> assertEquals(shelf, ytShelf(t), t) }
        assertNull(ytShelf(""))
    }

    private fun item(id: String) = ItemDto(id = id, type = "album", title = id)
    private fun sec(src: String, title: String, shelf: String?, vararg ids: String) = SectionDto(title, ids.map(::item), shelf).ns(src)

    private val yt = listOf(sec("yt", "Listen again", "continue", "y1", "y2"), sec("yt", "Jazz", null, "y3"))
    private val nd = listOf(sec("nd", "Continue listening", "continue", "n1", "n2"), sec("nd", "Recently added", "recent", "n3"))

    @Test
    fun mixedInterleavesTypedShelvesThenLeftovers() {
        val out = Mix.mixHome(listOf(yt, nd), separate = false)
        assertEquals(listOf("Continue listening", "Recently added", "Jazz"), out.map { it.title })
        assertEquals(listOf("y1", "nd:n1", "y2", "nd:n2").map { it.removePrefix("nd:") }, out[0].items.map { it.id.removePrefix("nd:") })
        assertNull(out[0].source)
        assertEquals("yt", out[2].source)
    }

    @Test
    fun separateKeepsEachSourcesSectionsWithTheirSource() {
        val out = Mix.mixHome(listOf(yt, nd), separate = true)
        assertEquals(listOf("Listen again", "Jazz", "Continue listening", "Recently added"), out.map { it.title })
        assertEquals(listOf("yt", "yt", "nd", "nd"), out.map { it.source })
        assertEquals("continue", out[0].shelf)
    }
}
