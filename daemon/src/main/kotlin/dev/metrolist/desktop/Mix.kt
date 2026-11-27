package dev.metrolist.desktop

import com.metrolist.music.utils.DesktopPaths
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.Json
import java.io.File

/**
 * What the client shows: the enabled sources merged into one Home, one Search and one Library.
 * With a single enabled source every call is a straight pass-through, so a YouTube-only setup behaves exactly as before.
 */
object Mix {
    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }
    private val snapshots get() = File(DesktopPaths.dataDir, "library")

    private fun live(): List<MusicSource> = Sources.configs().filter { it.enabled }.map { Sources.get(it.id) }

    /** Enabled sources that take part in search. */
    private fun searchable(): List<MusicSource> = Sources.configs().filter { it.enabled && it.search }.map { Sources.get(it.id) }

    /** Runs `block` on every source at once; a slow or failing source is a failed result, never the whole call. */
    private suspend fun <T> fan(srcs: List<MusicSource>, block: suspend (MusicSource) -> T): List<Pair<MusicSource, Result<T>>> = coroutineScope {
        srcs.map { s -> async { s to runCatching { withTimeout(12_000) { block(s) } } } }.awaitAll()
    }

    private fun <T> interleave(lists: List<List<T>>): List<T> =
        (0 until (lists.maxOfOrNull { it.size } ?: 0)).flatMap { i -> lists.mapNotNull { it.getOrNull(i) } }

    private val shelfTitles = linkedMapOf("continue" to "Continue listening", "recent" to "Recently added", "mixes" to "Mixes for you", "discover" to "Discover", "played" to "Most played")

    suspend fun home(layout: String = "mixed"): List<SectionDto> {
        // The saved flag lives in the registry; YouTube's own `config` is a fixed default.
        val onHome = Sources.configs().filter { it.home }.mapTo(HashSet()) { it.id }
        val srcs = live().filter { it.config.id in onHome }
        srcs.singleOrNull()?.let { s -> return s.home().map { it.ns(s.config.id) } }
        return mixHome(fan(srcs) { it.home() }.mapNotNull { (s, r) -> r.getOrNull()?.map { it.ns(s.config.id) } }, layout == "separate")
    }

    /** `got` is one list of source-tagged sections per participating source, in registry order. */
    internal fun mixHome(got: List<List<SectionDto>>, separate: Boolean): List<SectionDto> {
        if (separate) return got.flatten()
        // Shelves by intent mix items from every source; a source's own untyped sections follow as they are.
        val shelves = shelfTitles.mapNotNull { (shelf, title) ->
            val lists = got.map { secs -> secs.filter { it.shelf == shelf }.flatMap { it.items } }.filter { it.isNotEmpty() }
            if (lists.isEmpty()) null else SectionDto(title, interleave(lists).take(20), shelf)
        }
        return shelves + got.flatMap { secs -> secs.filter { it.shelf == null } }
    }

    /** Library sources first (what you own), then catalogues, each capped so one source cannot bury the rest. */
    suspend fun search(q: String, type: String): List<ItemDto> {
        val srcs = searchable().filter { Cap.SEARCH in it.caps }.sortedBy { it.kind != Kind.LIBRARY }
        srcs.singleOrNull()?.let { s -> return s.search(q, type).map { it.ns(s.config.id) } }
        val seen = HashSet<String>()
        return fan(srcs) { it.search(q, type) }
            .flatMap { (s, r) -> r.getOrDefault(emptyList()).take(20).map { it.ns(s.config.id) } }
            .filter { seen.add("${it.type}|${it.title.lowercase()}|${it.subtitle?.lowercase()}") }
    }

    /**
     * Search-as-you-type: YouTube's completions while it is enabled, plus quick matches from your own servers (cheap, and what a
     * self-hosted setup is searching for). Catalogue sources other than YouTube are left out: one request per keystroke is too much for them.
     */
    suspend fun suggest(q: String): SuggestDto {
        val yt = if (Sources.configs().any { it.id == "yt" && it.enabled && it.search }) runCatching { Services.suggest(q) }.getOrNull() else null
        val local = fan(searchable().filter { it.kind == Kind.LIBRARY && Cap.SEARCH in it.caps }) { s -> s.search(q, "album").take(3) + s.search(q, "artist").take(2) }
            .flatMap { (s, r) -> r.getOrDefault(emptyList()).map { it.ns(s.config.id) } }
        return SuggestDto(yt?.queries.orEmpty(), local + yt?.items.orEmpty().map { it.ns("yt") })
    }

    suspend fun songs(q: String): List<TrackDto> {
        if (searchable().singleOrNull() === YouTubeSource) return Services.search(q)
        return search(q, "song").map {
            TrackDto(it.id, it.title, it.subtitle?.split(", ")?.filter(String::isNotBlank).orEmpty(), it.album, it.durationSec, it.thumbnail,
                it.source ?: "yt", it.artistId, it.albumId, it.explicit, it.video)
        }
    }

    /**
     * Every configured source's library. A source that is off or unreachable still contributes its last snapshot,
     * marked unavailable with the reason, so the client can show those items greyed out instead of silently losing them.
     */
    suspend fun library(kind: String): List<ItemDto> {
        val rows = coroutineScope { Sources.configs().map { cfg -> async { row(cfg, kind) } }.awaitAll() }
        if (rows.all { it.first.isEmpty() }) rows.firstNotNullOfOrNull { it.second }?.let { throw it }
        fun key(i: ItemDto) = "${i.type}|${i.title.lowercase()}|${i.subtitle?.lowercase()}"
        val have = rows.flatMap { it.first }.filter { !it.unavailable }.mapTo(HashSet(), ::key)
        // A snapshot copy of something that is available elsewhere is redundant.
        return rows.flatMap { (items, _) -> items.filter { !it.unavailable || key(it) !in have } }
    }

    private suspend fun row(cfg: SourceConfig, kind: String): Pair<List<ItemDto>, Throwable?> {
        val file = File(snapshots, "${cfg.id}-$kind.json")
        fun cached(reason: String) = runCatching { json.decodeFromString(ListSerializer(ItemDto.serializer()), file.readText()) }.getOrDefault(emptyList())
            .map { it.copy(unavailable = true, reason = reason) }
        if (!cfg.enabled) return cached("${cfg.name} is turned off") to null
        val src = Sources.get(cfg.id)
        if (Cap.LIBRARY !in src.caps) return emptyList<ItemDto>() to null
        return try {
            val items = withTimeout(15_000) { src.library(kind) }.map { it.ns(cfg.id) }
            runCatching { snapshots.mkdirs(); file.writeText(json.encodeToString(ListSerializer(ItemDto.serializer()), items)) }
            items to null
        } catch (e: Throwable) {
            cached("Can't reach ${cfg.name}") to e
        }
    }

    /** Saves a source's library just before it is turned off, so its items can still be listed (greyed out) afterwards. */
    suspend fun snapshot(id: String) {
        val cfg = Sources.configs().firstOrNull { it.id == id && it.enabled } ?: return
        coroutineScope { listOf("playlists", "albums", "artists").map { kind -> async { row(cfg, kind) } }.awaitAll() }
    }

    fun forget(id: String) { snapshots.listFiles { f -> f.name.startsWith("$id-") }?.forEach { it.delete() } }
}
