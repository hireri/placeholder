plugins {
    id("org.jetbrains.kotlin.jvm")
    id("org.jetbrains.kotlin.plugin.serialization")
}

// Upstream's :innertube module, compiled unmodified (it has no Android dependencies).
sourceSets.main { kotlin.srcDir(rootDir.resolve("../innertube/src/main/kotlin")) }

dependencies {
    api(project(":shims"))
    api(libs.innertubex)
    api(libs.ktor.client.core)
    api(libs.ktor.client.okhttp)
    api(libs.ktor.client.content.negotiation)
    api(libs.ktor.client.encoding)
    api(libs.ktor.serialization.json)
}
