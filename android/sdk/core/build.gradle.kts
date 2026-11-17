// The universal core's Kotlin library. Everything it shares with the other modules lives in the
// convention plugin; this file names only what is its own.
plugins {
    id("craftbridge.android.library")
}

android {
    namespace = "io.github.christopherlinnett.craftbridge.core"
}

dependencies {
    // The generated bindings extend JNA's types in their public declarations, so an app compiles
    // against it too.
    api(libs.jna)
    api(libs.kotlinx.coroutines.core)
}
