package dev.metrolist.desktop

import java.util.concurrent.ConcurrentHashMap

/** Registry of configured sources, their order (also the de-dupe priority) and the id namespacing. */
object Sources {
    val store = CredentialStore()

    private val factories: Map<String, (SourceConfig) -> MusicSource> = mapOf(
        "navidrome" to ::NavidromeSource,
        "jellyfin" to ::JellyfinSource,
        "soundcloud" to ::SoundCloudSource,
        "bandcamp" to ::BandcampSource,
    )
    val types: Set<String> get() = factories.keys

    /** Providers with exactly one instance: listed from the start (off), never added or removed. */
    private val singletons = mapOf("soundcloud" to "SoundCloud", "bandcamp" to "Bandcamp")
    fun isSingleton(type: String) = type in singletons

    private val live = ConcurrentHashMap<String, MusicSource>()
    private val results = ConcurrentHashMap<String, TestResultDto>()

    private fun ytConfig() = store.load().let { YouTubeSource.config.copy(enabled = it.youtubeEnabled, home = it.youtubeHome, search = it.youtubeSearch) }

    /** Every source config in priority order; YouTube is always present. */
    fun configs(): List<SourceConfig> {
        val c = store.load()
        val seeded = singletons.filter { (t, _) -> t in factories && c.sources.none { it.type == t } }.map { (t, n) -> SourceConfig(id = t, type = t, name = n, enabled = false) }
        val all = (listOf(ytConfig()) + c.sources + seeded).associateBy { it.id }
        val ordered = c.sourceOrder.mapNotNull(all::get)
        return ordered + all.values.filter { it !in ordered }
    }

    /** Builds an unsaved source, for the add form's dry-run test. */
    fun build(cfg: SourceConfig): MusicSource = factories.getValue(cfg.type)(cfg)

    fun get(id: String): MusicSource {
        if (id == "yt") return YouTubeSource
        val cfg = configs().firstOrNull { it.id == id } ?: throw NoSuchElementException("no source $id")
        return live.getOrPut(id) { factories.getValue(cfg.type)(cfg) }
    }

    /** The source for a request: refuses a turned-off one so nothing reaches its service. */
    fun use(id: String): MusicSource {
        val cfg = configs().firstOrNull { it.id == id } ?: throw NoSuchElementException("no source $id")
        check(cfg.enabled) { "${cfg.name} is turned off" }
        return get(id)
    }

    /** The API id for a source's raw id (YouTube's stay bare). */
    fun fullId(src: String, raw: String) = if (src == "yt") raw else "$src:$raw"

    fun exists(id: String) = configs().any { it.id == id }

    /** `<sourceId>:<rawId>` to its parts; a bare id is YouTube. Raw ids may themselves contain ':' so only the first one splits. */
    fun parse(full: String): Pair<String, String> {
        val i = full.indexOf(':')
        return if (i > 0 && exists(full.substring(0, i))) full.substring(0, i) to full.substring(i + 1) else "yt" to full
    }

    fun status(cfg: SourceConfig): SourceStatusDto {
        val src = if (cfg.id == "yt") YouTubeSource else runCatching { get(cfg.id) }.getOrNull()
        val r = results[cfg.id]
        return SourceStatusDto(
            id = cfg.id, type = cfg.type, name = cfg.name, kind = (src?.kind ?: Kind.LIBRARY).name.lowercase(),
            caps = src?.caps.orEmpty().map { it.name.lowercase() }.sorted(), enabled = cfg.enabled, builtin = cfg.id == "yt",
            status = when { r == null -> "unchecked"; r.ok -> "ok"; else -> "error" }, error = r?.error, detail = r?.detail,
            url = cfg.url, username = cfg.username, scrobbleVia = cfg.scrobbleVia, home = cfg.home,
            search = cfg.search, singleton = isSingleton(cfg.type), secretSet = !cfg.secret.isNullOrEmpty(), avatarUrl = r?.avatar,
        )
    }

    suspend fun test(id: String): TestResultDto = get(id).test().also { results[id] = it }

    /** Throws IllegalArgumentException with a user-facing message for a bad request. */
    suspend fun add(req: SourceReq): SourceConfig {
        val type = req.type?.lowercase()?.takeIf { it in factories && !isSingleton(it) } ?: throw IllegalArgumentException("type must be one of ${(factories.keys.filter { !isSingleton(it) }).joinToString()}; SoundCloud and Bandcamp are built in")
        val cfg = SourceConfig(
            id = "$type-${java.util.UUID.randomUUID().toString().take(4)}", type = type,
            name = req.name?.takeIf { it.isNotBlank() } ?: type.replaceFirstChar { it.uppercase() },
            url = req.url?.trim()?.trimEnd('/')?.takeIf { it.isNotEmpty() }, username = req.username?.takeIf { it.isNotBlank() }, secret = req.secret?.takeIf { it.isNotEmpty() },
            enabled = req.enabled ?: true, scrobbleVia = req.scrobbleVia ?: "serafin",
        )
        store.update { it.copy(sources = it.sources + cfg, sourceOrder = (it.sourceOrder.ifEmpty { listOf("yt") } + cfg.id).distinct()) }
        return cfg
    }

    fun patch(id: String, req: SourceReq) {
        if (id == "yt") { store.update { c -> c.copy(youtubeEnabled = req.enabled ?: c.youtubeEnabled, youtubeHome = req.home ?: c.youtubeHome, youtubeSearch = req.search ?: c.youtubeSearch) }; return }
        req.scrobbleVia?.let { require(it == "serafin" || it == "server") { "scrobbleVia must be serafin or server" } }
        // A built-in provider is stored the first time it is changed.
        configs().firstOrNull { it.id == id }?.let { cfg -> store.update { c -> if (c.sources.any { it.id == id }) c else c.copy(sources = c.sources + cfg) } }
        store.update { c ->
            c.copy(sources = c.sources.map { s ->
                if (s.id != id) s else s.copy(
                    name = req.name?.takeIf { it.isNotBlank() } ?: s.name, url = req.url?.trim()?.trimEnd('/') ?: s.url, username = req.username ?: s.username,
                    secret = if (req.secret == "" && isSingleton(s.type)) null else req.secret?.takeIf { it.isNotEmpty() } ?: s.secret, enabled = req.enabled ?: s.enabled, scrobbleVia = req.scrobbleVia ?: s.scrobbleVia, home = req.home ?: s.home, search = req.search ?: s.search,
                )
            })
        }
        // Only a changed connection invalidates the live client and its last test.
        if (req.url != null || req.username != null || req.secret != null) { drop(id) }
    }

    private fun drop(id: String) { (live.remove(id) as? JellyfinSource)?.shutdown(); results.remove(id) }

    fun remove(id: String) {
        require(id != "yt") { "YouTube Music cannot be removed, only disabled" }
        require(configs().firstOrNull { it.id == id }?.type?.let(::isSingleton) != true) { "This source is built in and can only be turned off" }
        store.update { it.copy(sources = it.sources.filter { s -> s.id != id }, sourceOrder = it.sourceOrder - id) }
        drop(id)
        Mix.forget(id)
    }

    fun reorder(order: List<String>) = store.update { c ->
        val known = configs().map { it.id }
        c.copy(sourceOrder = (order.filter { it in known } + known).distinct())
    }
}

// Namespacing for non-YouTube sources: provider-raw ids out, `<sourceId>:<rawId>` in the API.
private fun String.ns(src: String) = if (src == "yt" || isEmpty()) this else "$src:$this"
@JvmName("nsOrNull") private fun String?.ns(src: String) = this?.ns(src)

fun ItemDto.ns(src: String) = copy(id = id.ns(src), artistId = artistId.ns(src), albumId = albumId.ns(src), source = src)
fun TrackDto.ns(src: String) = copy(id = id.ns(src), artistId = artistId.ns(src), albumId = albumId.ns(src))
fun SectionDto.ns(src: String) = copy(items = items.map { it.ns(src) }, source = src)
fun PageDto.ns(src: String) = copy(item = item.ns(src), tracks = tracks.map { it.ns(src) }, sections = sections.map { it.ns(src) })
