package dev.metrolist.desktop

import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.Json
import kotlin.test.Test
import kotlin.test.assertEquals

class SyncPlayTest {
    private fun obj(s: String) = Json.parseToJsonElement(s).jsonObject

    @Test fun ticksAndClockOffset() {
        assertEquals(1_500_0000, SyncPlay.msToTicks(1_500)); assertEquals(1_500, SyncPlay.ticksToMs(15_000_000))
        // server runs 1000 ms ahead, 40 ms each way
        assertEquals(1000, SyncPlay.clockOffset(0, 1040, 1040, 80))
    }

    @Test fun commandIsMappedToLocalTime() {
        val e = buildJsonObject(SyncPlay.commandEvent(obj("""{"Command":"Seek","When":"2024-01-01T00:00:10.000Z","PositionTicks":30000000,"PlaylistItemId":"p1"}"""), 2_000))
        val at = SyncPlay.parseInstantMs("2024-01-01T00:00:10.000Z")!! - 2_000
        assertEquals("seek", e["command"]!!.jsonPrimitive.content)
        assertEquals(3_000, e["positionMs"]!!.jsonPrimitive.content.toLong())
        assertEquals(at, e["atMs"]!!.jsonPrimitive.content.toLong())
    }

    @Test fun queueIdsAreNamespaced() {
        val e = buildJsonObject(SyncPlay.queueEvent(obj("""{"Playlist":[{"ItemId":"a","PlaylistItemId":"p1"},{"ItemId":"b","PlaylistItemId":"p2"}],"PlayingItemIndex":1,"StartPositionTicks":10000,"IsPlaying":true}"""), "jf"))
        assertEquals(listOf("jf:a", "jf:b"), e["ids"]!!.jsonArray.map { it.jsonPrimitive.content })
        assertEquals("p2", e["playlistItemIds"]!!.jsonArray[1].jsonPrimitive.content)
        assertEquals(1, e["index"]!!.jsonPrimitive.content.toInt()); assertEquals(1, e["positionMs"]!!.jsonPrimitive.content.toInt())
    }
}
