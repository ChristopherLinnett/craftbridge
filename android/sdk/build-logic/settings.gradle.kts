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

// The licence listing both Gradle sides print lives here, in a build of its own rather than in
// `:convention` or `:detekt-rules`, because a build script cannot load a project of its own build and
// each build reaches the other only through coordinates; a third build is the only code both can
// load. The folder carries no settings file — Gradle treats one without it as a single-project
// build — so the gate still reads exactly the two settings files AGENTS §10 allows.
includeBuild("licence-listing")
