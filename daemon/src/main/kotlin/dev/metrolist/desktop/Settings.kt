package dev.metrolist.desktop

import android.content.Context
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import com.metrolist.innertube.YouTube
import com.metrolist.innertube.models.YouTubeLocale
import com.metrolist.music.constants.AudioQuality
import com.metrolist.music.constants.AudioQualityKey
import com.metrolist.music.constants.ContentCountryKey
import com.metrolist.music.constants.ContentLanguageKey
import com.metrolist.music.constants.CountryCodeToName
import com.metrolist.music.constants.EnableBetterLyricsKey
import com.metrolist.music.constants.EnableKugouKey
import com.metrolist.music.constants.EnableLrcLibKey
import com.metrolist.music.constants.EnableLyricsPlus
import com.metrolist.music.constants.EnablePaxsenixKey
import com.metrolist.music.constants.EnableZemerKey
import com.metrolist.music.constants.LanguageCodeToName
import com.metrolist.music.constants.LyricsProviderOrderKey
import com.metrolist.music.constants.ProxyEnabledKey
import com.metrolist.music.constants.ProxyPasswordKey
import com.metrolist.music.constants.ProxyTypeKey
import com.metrolist.music.constants.ProxyUrlKey
import com.metrolist.music.constants.ProxyUsernameKey
import com.metrolist.music.constants.SYSTEM_DEFAULT
import com.metrolist.music.lyrics.LyricsProviderRegistry
import com.metrolist.music.utils.dataStore
import kotlinx.coroutines.flow.first
import kotlinx.serialization.Serializable
import okhttp3.Credentials
import java.net.Authenticator
import java.net.InetSocketAddress
import java.net.PasswordAuthentication
import java.net.Proxy
import java.util.Locale

/** Every field is optional: a PUT merges what is present, a GET fills them all. */
@Serializable
data class SettingsDto(
    val audioQuality: String? = null,
    val contentLanguage: String? = null,
    val contentCountry: String? = null,
    val proxy: ProxyDto? = null,
    val lyrics: LyricsSettingsDto? = null,
    val languages: List<OptionDto>? = null,
    val countries: List<OptionDto>? = null,
)

@Serializable data class ProxyDto(val enabled: Boolean? = null, val type: String? = null, val url: String? = null, val username: String? = null, val password: String? = null)

@Serializable data class LyricsSettingsDto(val providers: List<String>? = null, val enabled: Map<String, Boolean>? = null, val order: List<String>? = null)

@Serializable data class OptionDto(val code: String, val name: String)

object Settings {
    private val store = Context().dataStore

    // YouTube and YouTubeSubtitle have no Enable*Key upstream: always on.
    private val enableKeys = mapOf(
        "BetterLyrics" to EnableBetterLyricsKey,
        "Paxsenix" to EnablePaxsenixKey,
        "LrcLib" to EnableLrcLibKey,
        "KuGou" to EnableKugouKey,
        "LyricsPlus" to EnableLyricsPlus,
        "Zemer" to EnableZemerKey,
    )
    private val proxyTypes = setOf("HTTP", "SOCKS")
    private val languages = LanguageCodeToName.map { OptionDto(it.key, it.value) }
    private val countries = CountryCodeToName.map { OptionDto(it.key, it.value) }

    private suspend fun prefs() = store.data.first()

    suspend fun audioQuality(): AudioQuality =
        AudioQuality.entries.firstOrNull { it.name == prefs()[AudioQualityKey] } ?: AudioQuality.AUTO

    suspend fun lyricsOrder(): List<String> = LyricsProviderRegistry.deserializeProviderOrder(prefs()[LyricsProviderOrderKey].orEmpty())

    suspend fun get(): SettingsDto {
        val p = prefs()
        return SettingsDto(
            audioQuality = audioQuality().name,
            contentLanguage = p[ContentLanguageKey] ?: SYSTEM_DEFAULT,
            contentCountry = p[ContentCountryKey] ?: SYSTEM_DEFAULT,
            proxy = ProxyDto(
                enabled = p[ProxyEnabledKey] ?: false,
                type = p[ProxyTypeKey]?.takeIf { it in proxyTypes } ?: "HTTP",
                url = p[ProxyUrlKey] ?: "host:port",
                username = p[ProxyUsernameKey].orEmpty(),
                password = p[ProxyPasswordKey].orEmpty(),
            ),
            lyrics = LyricsSettingsDto(
                providers = LyricsProviderRegistry.getDefaultProviderOrder(),
                enabled = LyricsProviderRegistry.providerNames.associateWith { LyricsProviderRegistry.getProviderByName(it)!!.isEnabled(Services.context) },
                order = lyricsOrder(),
            ),
            languages = languages,
            countries = countries,
        )
    }

    /** Throws IllegalArgumentException, before anything is written, when any field is invalid. */
    suspend fun update(req: SettingsDto): SettingsDto {
        req.audioQuality?.let { q -> require(AudioQuality.entries.any { it.name == q }) { "unknown audioQuality: $q" } }
        req.contentLanguage?.let { require(it == SYSTEM_DEFAULT || it in LanguageCodeToName) { "unknown contentLanguage: $it" } }
        req.contentCountry?.let { require(it == SYSTEM_DEFAULT || it in CountryCodeToName) { "unknown contentCountry: $it" } }
        req.proxy?.type?.let { require(it in proxyTypes) { "unknown proxy type: $it" } }
        req.lyrics?.enabled?.forEach { (name, _) ->
            require(name in LyricsProviderRegistry.providerNames) { "unknown lyrics provider: $name" }
            require(name in enableKeys) { "lyrics provider $name cannot be disabled" }
        }
        req.lyrics?.order?.forEach { require(it in LyricsProviderRegistry.providerNames) { "unknown lyrics provider: $it" } }
        req.proxy?.let { p ->
            val cur = get().proxy!!
            if (p.enabled ?: cur.enabled == true) require(parseAddress(p.url ?: cur.url!!) != null) { "proxy url must be host:port" }
        }

        store.edit { e ->
            req.audioQuality?.let { e[AudioQualityKey] = it }
            req.contentLanguage?.let { e[ContentLanguageKey] = it }
            req.contentCountry?.let { e[ContentCountryKey] = it }
            req.proxy?.let { p ->
                p.enabled?.let { e[ProxyEnabledKey] = it }
                p.type?.let { e[ProxyTypeKey] = it }
                p.url?.let { e[ProxyUrlKey] = it }
                p.username?.let { e[ProxyUsernameKey] = it }
                p.password?.let { e[ProxyPasswordKey] = it }
            }
            req.lyrics?.enabled?.forEach { (name, on) -> e[enableKeys.getValue(name)] = on }
            req.lyrics?.order?.let { e[LyricsProviderOrderKey] = LyricsProviderRegistry.serializeProviderOrder(it) }
        }
        if (req.contentLanguage != null || req.contentCountry != null || req.proxy != null) apply()
        return get()
    }

    private fun parseAddress(url: String): InetSocketAddress? {
        val host = url.substringBeforeLast(':', "").takeIf { it.isNotBlank() } ?: return null
        val port = url.substringAfterLast(':').toIntOrNull()?.takeIf { it in 1..65535 } ?: return null
        return InetSocketAddress.createUnresolved(host, port)
    }

    /** Mirrors upstream App.kt initializeSettings: locale and proxy of YouTube from the stored preferences. */
    suspend fun apply() {
        val p: Preferences = prefs()
        val locale = Locale.getDefault()
        YouTube.locale = YouTubeLocale(
            gl = p[ContentCountryKey]?.takeIf { it != SYSTEM_DEFAULT } ?: locale.country.takeIf { it in CountryCodeToName } ?: "US",
            hl = p[ContentLanguageKey]?.takeIf { it != SYSTEM_DEFAULT } ?: locale.language.takeIf { it in LanguageCodeToName } ?: "en",
        )

        val type = if (p[ProxyTypeKey] == "SOCKS") Proxy.Type.SOCKS else Proxy.Type.HTTP
        val address = if (p[ProxyEnabledKey] == true) p[ProxyUrlKey]?.let(::parseAddress) else null
        val username = p[ProxyUsernameKey].orEmpty()
        val password = p[ProxyPasswordKey].orEmpty()
        val auth = address != null && (username.isNotEmpty() || password.isNotEmpty())
        YouTube.proxyAuth = if (auth && type == Proxy.Type.HTTP) Credentials.basic(username, password) else null
        Authenticator.setDefault(
            if (auth && type == Proxy.Type.SOCKS) object : Authenticator() {
                override fun getPasswordAuthentication() = PasswordAuthentication(username, password.toCharArray())
            } else null,
        )
        YouTube.proxy = address?.let { Proxy(type, it) }
    }
}
