package dev.metrolist.desktop

import com.metrolist.music.utils.DesktopPaths
import kotlinx.serialization.json.Json
import java.io.File
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.nio.file.attribute.PosixFilePermissions

/** Plain JSON file, owner-only. ponytail: OS keyring (libsecret/Keychain/DPAPI) if this needs hardening. */
class CredentialStore(private val file: File = File(DesktopPaths.dataDir, "credentials.json")) {
    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }

    /**
     * A file whose contents can't be parsed is set aside, never overwritten, so a later save can't wipe the logins in it.
     * Anything else (a read error, a class that failed to load) is thrown: the file is fine, and treating it as empty
     * would sign the user out of everything.
     */
    @Synchronized
    fun load(): StoredCreds {
        if (!file.exists()) return StoredCreds()
        val text = file.readText()
        return try {
            json.decodeFromString<StoredCreds>(text)
        } catch (e: IllegalArgumentException) { // SerializationException is one
            val bad = File(file.parentFile, "${file.name}.bad-${System.currentTimeMillis()}")
            System.err.println("credentials: ${file.name} is unreadable (${e.message?.substringBefore("JSON input")}); kept as ${bad.name}")
            file.renameTo(bad)
            StoredCreds()
        }
    }

    /** Written owner-only to a temp file and renamed over the old one, so a crash mid-write leaves the previous file intact. */
    @Synchronized
    fun save(creds: StoredCreds) {
        val dir = file.absoluteFile.parentFile.toPath()
        val tmp = runCatching { Files.createTempFile(dir, file.name, ".tmp", PosixFilePermissions.asFileAttribute(PosixFilePermissions.fromString("rw-------"))) }
            .getOrElse { Files.createTempFile(dir, file.name, ".tmp") } // no POSIX permissions on Windows
        try {
            Files.writeString(tmp, json.encodeToString(StoredCreds.serializer(), creds))
            Files.move(tmp, file.toPath(), StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE)
        } finally {
            Files.deleteIfExists(tmp)
        }
    }

    @Synchronized
    fun update(block: (StoredCreds) -> StoredCreds) = save(block(load()))
}
