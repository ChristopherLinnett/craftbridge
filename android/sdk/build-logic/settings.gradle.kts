// The convention-plugin build. It reads the same catalog the SDK modules do, so a
// plugin can name a plugin or tool by version without ever repeating a literal. Its one
// plugin, kotlin-dsl, comes from the Gradle Plugin Portal, where Gradle looks by default.

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)

    repositories {
        google()
        mavenCentral()
    }

    versionCatalogs {
        create("libs") {
            from(files("../gradle/libs.versions.toml"))
        }
    }
}

rootProject.name = "build-logic"

include(":convention")
