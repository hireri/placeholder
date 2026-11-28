package dev.metrolist.desktop

import java.nio.file.Files
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class CredentialStoreTest {
    @Test
    fun unreadableFileIsKeptAndSavesReplaceAtomically() {
        val dir = Files.createTempDirectory("creds").toFile()
        val file = dir.resolve("credentials.json").apply { writeText("{not json") }
        val store = CredentialStore(file)
        store.update { it.copy(youtubeEnabled = false) }
        val bad = dir.listFiles()!!.single { it.name.startsWith("credentials.json.bad-") }
        assertEquals("{not json", bad.readText())
        assertEquals(false, store.load().youtubeEnabled)
        assertEquals(setOf("credentials.json", bad.name), dir.list()!!.toSet()) // no temp file left behind
        if ("posix" in java.nio.file.FileSystems.getDefault().supportedFileAttributeViews()) assertTrue(Files.getPosixFilePermissions(file.toPath()).map { it.name }.all { it.startsWith("OWNER") })
        dir.deleteRecursively()
    }
}
