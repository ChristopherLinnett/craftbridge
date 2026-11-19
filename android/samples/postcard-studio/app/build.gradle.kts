// Postcard Studio, the running example as an Android app. Everything it shares with the SDK's modules lives in
// the convention plugin; this file names only what is its own.
plugins {
    id("craftbridge.android.application")
}

android {
    namespace = "io.github.christopherlinnett.craftbridge.postcardstudio"
}

dependencies {
    // The SDK by its coordinates, as an app takes it; the settings answer them with the SDK's own modules.
    implementation("io.github.christopherlinnett.craftbridge:effects")
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
}
