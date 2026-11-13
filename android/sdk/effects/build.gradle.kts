// The EffectCraft plugin's Kotlin library, built on the universal core. Everything it shares with
// the other modules lives in the convention plugin; this file names only what is its own.
plugins {
    id("craftbridge.android.library")
}

android {
    namespace = "io.github.christopherlinnett.craftbridge.effects"
}

dependencies {
    // The core's types appear in this module's API, so apps get the core with it.
    api(project(":core"))
}
