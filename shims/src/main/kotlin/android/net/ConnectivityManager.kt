package android.net

/** Desktop is never treated as metered. */
open class ConnectivityManager {
    open val isActiveNetworkMetered: Boolean = false
}
