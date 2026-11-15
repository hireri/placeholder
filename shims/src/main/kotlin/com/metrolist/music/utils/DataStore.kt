package com.metrolist.music.utils

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.Preferences
import kotlinx.coroutines.flow.first
import java.io.File

/**
 * Desktop replacement for upstream utils/DataStore.kt (which is Compose + Android bound).
 * Same names, so unmodified upstream code that reads settings compiles against it.
 */
object DesktopPaths {
    val dataDir: File = File(
        System.getProperty("metrolist.dataDir")
            ?: (System.getenv("XDG_CONFIG_HOME") ?: "${System.getProperty("user.home")}/.config") + "/metrolist-desktop",
    ).also { it.mkdirs() }
}

private val store: DataStore<Preferences> by lazy {
    PreferenceDataStoreFactory.create(produceFile = { File(DesktopPaths.dataDir, "settings.preferences_pb") })
}

@Suppress("UnusedReceiverParameter")
val Context.dataStore: DataStore<Preferences> get() = store

operator fun <T> DataStore<Preferences>.get(key: Preferences.Key<T>): T? =
    kotlinx.coroutines.runBlocking { data.first()[key] }
