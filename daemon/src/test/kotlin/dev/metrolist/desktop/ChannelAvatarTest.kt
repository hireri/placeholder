package dev.metrolist.desktop

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class ChannelAvatarTest {
    @Test
    fun readsTheAvatarAsAResizableSquare() {
        val json = """{"metadata":{"channelMetadataRenderer":{"avatar":{"thumbnails":[{"url":"https://yt3.googleusercontent.com/abc=s900-c-k-no-rj"}]}}}}"""
        assertEquals("https://yt3.googleusercontent.com/abc=w1200-h1200-l90-rj", ChannelAvatar.parse(json))
        assertNull(ChannelAvatar.parse("""{"alerts":[]}"""))
    }
}
