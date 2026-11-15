package com.metrolist.music.utils.potoken

import android.content.Context

/**
 * Upstream mints web PoTokens in an Android WebView. There is none here, so this returns no
 * token and the extractor falls back to clients that don't need one. A future bridge to the
 * client's wry window can fill this in without touching upstream code.
 */
class PoTokenGenerator(@Suppress("UNUSED_PARAMETER") context: Context) {
    suspend fun getWebClientPoToken(videoId: String, sessionId: String): PoTokenResult? = null

    suspend fun close() = Unit
}
