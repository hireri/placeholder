package dev.metrolist.desktop

import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonObject
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNotNull
import kotlin.test.assertNull

class SpotifyAuthTest {
    @Test
    fun cookieEraCredentialsStillLoadAndKeepOtherLogins() {
        val file = File.createTempFile("creds", ".json").apply {
            writeText("""{"youtube":{"cookie":"c"},"spotify":{"spDc":"old"}}""")
            deleteOnExit()
        }
        val creds = CredentialStore(file).load()
        assertEquals("c", creds.youtube?.cookie)
        assertNull(creds.spotify?.login)
    }

    @Test
    fun savedCredentialsStayReadableByOlderReleases() {
        val file = File.createTempFile("creds", ".json").apply { delete(); deleteOnExit() }
        CredentialStore(file).save(StoredCreds(spotify = SpotifyCreds(SpotifyLogin("device", JsonObject(emptyMap())))))
        assert("\"spDc\":\"\"" in file.readText())
    }

    @Test
    fun onlyARefusedLoginIsForgotten() = runBlocking {
        val login = SpotifyLogin("device", JsonObject(emptyMap()))
        var answer = 1 to ""
        var runs = 0
        SpotifyAuth.mint = { runs++; answer }

        SpotifyAuth.set(login)
        assertFailsWith<IllegalStateException> { SpotifyAuth.tokens() } // offline: still signed in
        assertFailsWith<IllegalStateException> { SpotifyAuth.tokens() }
        assertEquals(1, runs) // and the helper is not started again straight away
        assertEquals(true, SpotifyAuth.signedIn)
        SpotifyAuth.retryAfterMs = 0
        answer = 0 to """{"bearer":"b","clientToken":"c","expiresIn":3600}"""
        assertEquals("b", assertNotNull(SpotifyAuth.tokens()).bearer)
        SpotifyAuth.tokens()
        assertEquals(2, runs) // cached

        SpotifyAuth.set(SpotifyLogin("device", JsonObject(emptyMap())))
        answer = 2 to ""
        assertNull(SpotifyAuth.tokens())
        assertNull(SpotifyAuth.tokens())
        assertEquals(3, runs) // a refused login is not retried
        assertEquals(false, SpotifyAuth.signedIn)
        SpotifyAuth.set(null)
        SpotifyAuth.retryAfterMs = 30_000
    }

    @Test
    fun rateLimitsBackOffAndKeepWhatWasAlreadyRead() = runBlocking {
        SpotifyAuth.set(null)
        assertEquals("kept", SpotifyAuth.gated("k") { "kept" })
        assertEquals(listOf(30L, 60L, 120L, 240L, 300L, 300L), List(6) { SpotifyAuth.rateLimited(null) })
        assertEquals(7L, SpotifyAuth.rateLimited(7))
        assertEquals("kept", SpotifyAuth.gated<String>("k") { error("not called") })
        assertFailsWith<IllegalStateException> { SpotifyAuth.gated("new") { "sent while blocked" } }
        SpotifyAuth.set(null)
    }
}
