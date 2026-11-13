// The tools every module's convention plugin applies, put on the build's classpath once, here,
// at the versions the catalog pins. Declaring them at the root lets the convention plugins in
// build-logic use their typed APIs, and keeps every module's build file free of tool requests.
// AGP 9 compiles Kotlin itself: the Kotlin plugin is only on the classpath, to pin its version,
// and is never applied.
plugins {
    alias(libs.plugins.android.library) apply false
    alias(libs.plugins.kotlin.android) apply false
    alias(libs.plugins.spotless) apply false
    // Formats this build's own scripts and the convention plugins in build-logic.
    id("craftbridge.quality")
}
