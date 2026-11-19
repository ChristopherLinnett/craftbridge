package io.github.christopherlinnett.craftbridge.convention

import com.android.build.api.dsl.LibraryExtension
import org.gradle.api.Project
import org.gradle.kotlin.dsl.configure

/**
 * The Android library setup every CraftBridge SDK module shares: what every Android module shares, and what only a
 * published library has: the consumer R8 rules its AAR carries and the publication of the AAR (plan §23).
 */
class AndroidLibraryConventionPlugin : AndroidModuleConventionPlugin(AndroidModuleKind.LIBRARY, "com.android.library") {
    override fun Project.configureKind() {
        configurePublishing()
        extensions.configure<LibraryExtension> {
            defaultConfig {
                // Published inside the AAR, so an app that shrinks its code keeps what the native
                // bindings need without knowing the details.
                consumerProguardFiles("consumer-rules.pro")
            }
        }
    }
}
