plugins {
    // AGP 9+ has built-in Kotlin support; no Kotlin plugin needed.
    id("com.android.application")
}

// Build with -PtargetSdk=34 to produce a targetSdk-34 variant of the same code.
val targetSdkValue: Int = (providers.gradleProperty("targetSdk").orNull ?: "37").toInt()

android {
    namespace = "com.example.fgslab"
    compileSdk = 37
    buildToolsVersion = "37.0.0"

    defaultConfig {
        applicationId = "com.example.fgslab"
        minSdk = 29
        targetSdk = targetSdkValue
        versionCode = 1
        versionName = "1.0-ts$targetSdkValue"
    }

    // fgsTypes fills android:foregroundServiceType of the one <service> in the main
    // manifest; each flavor's own manifest adds only its per-type permissions.
    flavorDimensions += "variant"
    productFlavors {
        // Declares every type under test, plus all the permissions and the specialUse property.
        create("full") {
            dimension = "variant"
            manifestPlaceholders["fgsTypes"] = "dataSync|mediaProcessing|specialUse|shortService"
        }
        // The candidate single-APK declaration: mediaProcessing|dataSync.
        create("combo") {
            dimension = "variant"
            manifestPlaceholders["fgsTypes"] = "mediaProcessing|dataSync"
        }
        // No foregroundServiceType at all: its manifest removes the attribute. The merger
        // still needs a placeholder value before it applies the removal.
        create("none") {
            dimension = "variant"
            manifestPlaceholders["fgsTypes"] = ""
        }
        // Types declared but per-type permissions NOT requested (no flavor manifest),
        // to capture the exact SecurityException.
        create("noperm") {
            dimension = "variant"
            manifestPlaceholders["fgsTypes"] = "dataSync|mediaProcessing|specialUse"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
