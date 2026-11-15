package android.content.pm

class PackageInfo(@JvmField var versionName: String? = null)

open class PackageManager {
    open fun getPackageInfo(packageName: String, flags: Int): PackageInfo =
        PackageInfo(System.getProperty("metrolist.version") ?: "desktop")
}
