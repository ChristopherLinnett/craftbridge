pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

// The SDK's own convention plugins and detekt rules, so this app's sources are held to the same formatting,
// Lint, compiler and detekt rules as the SDK's modules, by the same code. Included at the settings level so
// the detekt rules' project is substituted into the dependency the convention plugin asks for by coordinate.
includeBuild("../../sdk/build-logic")

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
    // The one catalog of the repository: every version is written there once.
    versionCatalogs {
        create("libs") {
            from(files("../../sdk/gradle/libs.versions.toml"))
        }
    }
}

rootProject.name = "postcard-studio"

// The SDK, taken as an app takes it: by its Maven coordinates, which an included build answers with its own
// modules in place of a published release.
includeBuild("../../sdk") {
    dependencySubstitution {
        substitute(module("io.github.christopherlinnett.craftbridge:core")).using(project(":core"))
        substitute(module("io.github.christopherlinnett.craftbridge:effects")).using(project(":effects"))
    }
}

include(":app")
