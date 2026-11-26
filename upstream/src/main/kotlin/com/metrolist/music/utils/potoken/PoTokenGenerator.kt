package com.metrolist.music.utils.potoken

import android.content.Context

/**
 * Upstream mints web PoTokens in an Android WebView. The desktop has none, so the daemon installs
 * [provider] (a hidden webview in the serafin-potoken helper). Unset, no token is returned and the
 * extractor falls back to clients that don't need one.
 */
class PoTokenGenerator(@Suppress("UNUSED_PARAMETER") context: Context) {
    suspend fun getWebClientPoToken(videoId: String, sessionId: String): PoTokenResult? = provider?.invoke(videoId, sessionId)

    suspend fun close() = Unit

    companion object {
        @Volatile
        var provider: (suspend (videoId: String, sessionId: String) -> PoTokenResult?)? = null
    }
}
