plugins {
    id("org.jetbrains.kotlin.jvm")
    id("org.jetbrains.kotlin.plugin.serialization")
    application
}

application { mainClass.set("dev.metrolist.desktop.MainKt") }

val ktor = libs.versions.ktor.get()

dependencies {
    implementation(project(":upstream"))
    implementation(project(":bitchord"))
    implementation("io.ktor:ktor-server-core:$ktor")
    implementation("io.ktor:ktor-server-cio:$ktor")
    implementation("io.ktor:ktor-server-content-negotiation:$ktor")
    implementation("io.ktor:ktor-server-status-pages:$ktor")
    testImplementation(kotlin("test"))
}
