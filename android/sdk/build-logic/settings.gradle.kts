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

include(":convention", ":detekt-rules")

// The code both sides of this build load — the licence listing both Gradle sides print, and the
// written-against-run test count the convention plugin and `:detekt-rules` both wire — lives here, in a
// build of its own rather than in `:convention` or `:detekt-rules`, because a build script cannot load
// a project of its own build and each build reaches the other only through coordinates; a third build
// is the only code both can load. It is one build for all such code, never one per piece: a second
// build would repeat this one's build file, and an applied script cannot hold what they share, since
// it compiles without the plugin classes of the build that applies it. The folder carries no settings
// file — Gradle treats one without it as a single-project build — so the gate still reads exactly the
// two settings files AGENTS §10 allows.
includeBuild("shared")
