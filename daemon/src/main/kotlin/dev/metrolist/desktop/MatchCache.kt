package dev.metrolist.desktop

import com.metrolist.music.utils.DesktopPaths
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.io.File
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

@Serializable
internal data class CachedMatch(val track: TrackDto? = null, val at: Long = 0L)

@Serializable
private data class MatchCacheFile(val entries: Map<String, CachedMatch> = emptyMap())

/** Persisted Spotify-id -> YouTube-match cache; misses are kept briefly so failures are not retried in a loop. */
internal class MatchCache(private val file: File = File(DesktopPaths.dataDir, "spotify-matches.json")) {
    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }
    private val entries = ConcurrentHashMap<String, CachedMatch>()
    private val saver = Executors.newSingleThreadScheduledExecutor { r ->
        Thread(r, "match-cache-save").also { it.isDaemon = true }
    }

    init {
        runCatching {
            json.decodeFromString<MatchCacheFile>(file.readText()).entries.forEach { (k, v) -> entries[k] = v }
        }
        Runtime.getRuntime().addShutdownHook(Thread({ runCatching(::save) }, "match-cache-flush"))
    }

    /** Null means no usable entry; a hit with a null `track` is a cached no-match. */
    fun get(key: String): CachedMatch? {
        val hit = entries[key] ?: return null
        val ttl = if (hit.track == null) MISS_TTL_MS else HIT_TTL_MS
        if (System.currentTimeMillis() - hit.at > ttl) {
            entries.remove(key, hit)
            return null
        }
        return hit
    }

    fun put(key: String, track: TrackDto?) {
        entries[key] = CachedMatch(track, System.currentTimeMillis())
        scheduleSave()
    }

    fun saveNow() = save()

    private fun scheduleSave() {
        saver.schedule({ runCatching(::save) }, SAVE_DEBOUNCE_MS, TimeUnit.MILLISECONDS)
    }

    @Synchronized
    private fun save() {
        file.parentFile?.mkdirs()
        val tmp = File(file.parentFile, "${file.name}.tmp")
        tmp.writeText(json.encodeToString(MatchCacheFile.serializer(), MatchCacheFile(entries.toMap())))
        runCatching {
            Files.move(tmp.toPath(), file.toPath(), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
        }.onFailure {
            runCatching { Files.move(tmp.toPath(), file.toPath(), StandardCopyOption.REPLACE_EXISTING) }
        }
    }

    private companion object {
        const val HIT_TTL_MS = 30L * 24 * 60 * 60 * 1000
        const val MISS_TTL_MS = 24L * 60 * 60 * 1000
        const val SAVE_DEBOUNCE_MS = 2_000L
    }
}
