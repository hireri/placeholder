package dev.metrolist.desktop

import kotlin.test.Test
import kotlin.test.assertEquals

class SpotifyAuthTest {
    // Expected value from an independent Python HMAC-SHA1 implementation of the same algorithm.
    @Test
    fun totpMatchesReference() {
        val codes = listOf(107, 81, 49, 57, 67, 93, 87, 81, 69, 67, 40, 93, 48, 50, 46, 91, 94, 113, 41, 47)
        assertEquals("657922", SpotifyAuth.totp(codes, 1_760_000_000_000))
    }
}
