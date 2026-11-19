// The tools the app's convention plugin applies, put on the build's classpath once, here, at the versions the
// catalog pins, as the SDK's own root build file does.
plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.kotlin.android) apply false
    alias(libs.plugins.kotlin.compose) apply false
    alias(libs.plugins.spotless) apply false
    // Formats this build's own scripts.
    id("craftbridge.quality")
}
