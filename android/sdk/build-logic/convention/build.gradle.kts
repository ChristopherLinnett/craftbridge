// Convention plugins: the one place where the modules' shared setup is written. A module
// build file applies these instead of repeating configuration, which is what keeps two
// modules from drifting apart (AGENTS.md "No copy-paste").

plugins {
    `kotlin-dsl`
}

group = "io.github.christopherlinnett.craftbridge.build"

kotlin {
    // The plugins run inside the Gradle daemon, so they compile against the JDK Gradle
    // itself runs on (JDK 21 here), not against a downloaded toolchain.
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_21
    }
}

dependencies {
    // compileOnly so the plugins compile against the real plugin APIs, but their jars are
    // never shipped to the modules' runtime classpath: Gradle puts one copy of each plugin
    // on a build's classpath, by plugin id, at the version the catalog pins.
    compileOnly(libs.android.gradle.plugin)
    compileOnly(libs.kotlin.gradle.plugin)
    compileOnly(libs.spotless.gradle.plugin)
}

gradlePlugin {
    plugins {
        register("androidLibrary") {
            id = "craftbridge.android.library"
            implementationClass = "io.github.christopherlinnett.craftbridge.convention.AndroidLibraryConventionPlugin"
        }
        register("quality") {
            id = "craftbridge.quality"
            implementationClass = "io.github.christopherlinnett.craftbridge.convention.QualityConventionPlugin"
        }
    }
}
