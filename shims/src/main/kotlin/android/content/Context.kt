package android.content

import android.content.pm.PackageManager
import com.metrolist.music.utils.DesktopPaths
import java.io.File
import java.util.Properties

/**
 * Minimal desktop stand-in for android.content.Context. Upstream code only ever asks it for
 * app metadata, shared preferences, or passes it through to preference lookups; widen it only
 * when the compiler asks.
 */
open class Context {
    open val packageName: String = "com.metrolist.desktop"
    open val packageManager: PackageManager = PackageManager()
    open val applicationContext: Context get() = this

    private val prefs = HashMap<String, SharedPreferences>()

    @Synchronized
    open fun getSharedPreferences(name: String, mode: Int): SharedPreferences =
        prefs.getOrPut(name) { SharedPreferences(File(DesktopPaths.dataDir, "$name.properties")) }

    companion object {
        const val MODE_PRIVATE = 0
    }
}

/** Properties-file backed SharedPreferences, enough for the string/long/boolean reads upstream does. */
class SharedPreferences(private val file: File) {
    private val props = Properties().apply { if (file.exists()) file.inputStream().use(::load) }

    @Synchronized fun getString(key: String, default: String?): String? = props.getProperty(key, default)
    @Synchronized fun getLong(key: String, default: Long): Long = props.getProperty(key)?.toLongOrNull() ?: default
    @Synchronized fun getBoolean(key: String, default: Boolean): Boolean = props.getProperty(key)?.toBooleanStrictOrNull() ?: default

    fun edit() = Editor()

    inner class Editor {
        private val pending = LinkedHashMap<String, String?>()
        fun putString(key: String, value: String?) = apply { pending[key] = value }
        fun putLong(key: String, value: Long) = apply { pending[key] = value.toString() }
        fun putBoolean(key: String, value: Boolean) = apply { pending[key] = value.toString() }
        fun remove(key: String) = apply { pending[key] = null }
        fun apply() = synchronized(this@SharedPreferences) {
            pending.forEach { (k, v) -> if (v == null) props.remove(k) else props.setProperty(k, v) }
            file.outputStream().use { props.store(it, null) }
        }
    }
}
