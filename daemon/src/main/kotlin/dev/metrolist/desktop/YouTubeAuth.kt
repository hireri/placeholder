package dev.metrolist.desktop

import com.metrolist.innertube.YouTube

/** Mirrors what upstream App.kt does when the stored cookie / visitorData / dataSyncId change. */
object YouTubeAuth {
    suspend fun apply(creds: YouTubeCreds?) {
        if (creds == null) {
            YouTube.cookie = null
            YouTube.dataSyncId = null
            return
        }
        YouTube.useLoginForBrowse = true
        YouTube.cookie = creds.cookie
        YouTube.visitorData = creds.visitorData?.takeIf { it.isNotBlank() && it != "null" }
            ?: YouTube.visitorData().getOrNull()
        YouTube.dataSyncId = creds.dataSyncId?.let {
            // same normalisation as upstream App.kt
            it.takeIf { v -> !v.contains("||") } ?: it.takeIf { v -> v.endsWith("||") }?.substringBefore("||") ?: it.substringAfter("||")
        }
    }

    suspend fun status(creds: YouTubeCreds?): ServiceStatus {
        if (creds == null) return ServiceStatus(false)
        return YouTube.accountInfo().fold(
            onSuccess = { ServiceStatus(true, UserDto(it.name, it.email ?: it.channelHandle, it.thumbnailUrl)) },
            onFailure = { ServiceStatus(false, error = it.message ?: it::class.simpleName) },
        )
    }
}
