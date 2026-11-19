package io.github.christopherlinnett.craftbridge.convention

import com.android.build.api.dsl.ApplicationExtension
import com.android.build.api.variant.ApplicationAndroidComponentsExtension
import org.gradle.api.Project
import org.gradle.kotlin.dsl.configure

/**
 * The Android application setup every CraftBridge sample shares: what every Android module shares
 * ([configureAndroidModule]), Compose's compiler, and the running example's template and sample photo as assets. A sample's
 * Kotlin is held to the same formatting, Lint, compiler and detekt rules as the SDK's modules, which is why it
 * is built by this build-logic and no build file of its own.
 */
class AndroidApplicationConventionPlugin : AndroidModuleConventionPlugin(AndroidModuleKind.APPLICATION, "com.android.application") {
    override fun Project.configureKind() {
        pluginManager.apply("org.jetbrains.kotlin.plugin.compose")
        extensions.configure<ApplicationExtension> {
            defaultConfig.targetSdk = catalogVersion("android-compileSdk").toInt()
            defaultConfig.versionCode = 1
            defaultConfig.versionName = "1"
            // `lintRelease` is the gate's own release Lint run, so the one Lint runs again on `assembleRelease`
            // would only be a second run of it, which Gradle reports as skipped, and a skipped task fails the gate.
            lint.checkReleaseBuilds = false
        }
        // A sample is checked by its unit tests and by the spine run on a device; a test APK none of the gate's
        // tasks build would be an empty component whose tasks Gradle reports as skipped.
        extensions.configure<ApplicationAndroidComponentsExtension> {
            beforeVariants { variant -> variant.androidTestEnabled = false }
        }
        forEachComponent(main = { variant -> bundleSampleAssets(variant) }, test = {})
    }
}
