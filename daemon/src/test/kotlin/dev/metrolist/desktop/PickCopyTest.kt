package dev.metrolist.desktop

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class PickCopyTest {
    private fun t(id: String, title: String, secs: Int) = TrackDto(id, title, listOf("Artist"), durationSec = secs, source = "nd")

    @Test
    fun takesOnlyTheSameEdit() {
        val hits = listOf(t("long", "Song", 215), t("live", "Song (Live)", 200), t("same", "Song", 202))
        assertEquals("same", Services.pickCopy(hits, "Song", "Artist", 200)?.id)
        assertNull(Services.pickCopy(hits.dropLast(1), "Song", "Artist", 200))
    }
}
