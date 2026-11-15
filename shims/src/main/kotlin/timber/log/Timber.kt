package timber.log

/** Source-compatible stand-in for the Android-only Timber API used by upstream code. */
object Timber {
    @Volatile var enabled: Boolean = System.getenv("METROLIST_LOG") != null

    private fun log(level: String, tag: String?, msg: String?, t: Throwable?, args: Array<out Any?>) {
        if (!enabled) return
        val text = if (msg != null && args.isNotEmpty()) msg.format(*args) else msg
        System.err.println("$level/${tag ?: "-"}: $text")
        t?.printStackTrace()
    }

    fun tag(tag: String): Tree = Tree(tag)
    fun v(msg: String?, vararg args: Any?) = log("V", null, msg, null, args)
    fun d(msg: String?, vararg args: Any?) = log("D", null, msg, null, args)
    fun i(msg: String?, vararg args: Any?) = log("I", null, msg, null, args)
    fun w(msg: String?, vararg args: Any?) = log("W", null, msg, null, args)
    fun e(msg: String?, vararg args: Any?) = log("E", null, msg, null, args)
    fun d(t: Throwable?, msg: String? = null, vararg args: Any?) = log("D", null, msg, t, args)
    fun w(t: Throwable?, msg: String? = null, vararg args: Any?) = log("W", null, msg, t, args)
    fun e(t: Throwable?, msg: String? = null, vararg args: Any?) = log("E", null, msg, t, args)

    class Tree(private val tag: String) {
        fun v(msg: String?, vararg args: Any?) = log("V", tag, msg, null, args)
        fun d(msg: String?, vararg args: Any?) = log("D", tag, msg, null, args)
        fun i(msg: String?, vararg args: Any?) = log("I", tag, msg, null, args)
        fun w(msg: String?, vararg args: Any?) = log("W", tag, msg, null, args)
        fun e(msg: String?, vararg args: Any?) = log("E", tag, msg, null, args)
        fun d(t: Throwable?, msg: String? = null, vararg args: Any?) = log("D", tag, msg, t, args)
        fun w(t: Throwable?, msg: String? = null, vararg args: Any?) = log("W", tag, msg, t, args)
        fun e(t: Throwable?, msg: String? = null, vararg args: Any?) = log("E", tag, msg, t, args)
    }
}
