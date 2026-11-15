package dev.metrolist.desktop

import com.metrolist.music.utils.DesktopPaths
import kotlinx.serialization.json.Json
import java.io.File
import java.nio.file.Files
import java.nio.file.attribute.PosixFilePermissions

/** Plain JSON file, owner-only. ponytail: OS keyring (libsecret/Keychain/DPAPI) if this needs hardening. */
class CredentialStore(private val file: File = File(DesktopPaths.dataDir, "credentials.json")) {
    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }

    @Synchronized
    fun load(): StoredCreds = runCatching { json.decodeFromString<StoredCreds>(file.readText()) }.getOrDefault(StoredCreds())

    @Synchronized
    fun save(creds: StoredCreds) {
        file.writeText(json.encodeToString(StoredCreds.serializer(), creds))
        runCatching { Files.setPosixFilePermissions(file.toPath(), PosixFilePermissions.fromString("rw-------")) }
    }

    @Synchronized
    fun update(block: (StoredCreds) -> StoredCreds) = save(block(load()))
}
