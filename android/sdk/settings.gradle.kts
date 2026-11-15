import org.gradle.api.artifacts.ProjectDependency
import org.gradle.api.artifacts.component.ProjectComponentIdentifier

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

// Included at the settings level rather than inside pluginManagement: the modules take the build's own
// detekt rules on their rule classpath by coordinate, and only a settings-level inclusion substitutes
// an included build's projects into ordinary dependencies — a pluginManagement inclusion substitutes
// plugin ids alone.
includeBuild("build-logic")

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "craftbridge"

include(":core", ":effects")

// Every module gets its formatting, Lint and Kotlin rules from the convention plugin, which the gate
// relies on: a module that applied the Android plugin itself would build without any of them. The same
// hook settles the dependency direction (AGENTS §2) on Gradle's own project graph: `:core` depends on
// no module of this build — the universal core knows nothing about any Craft app — and a plugin module
// may depend on `:core` alone. A typed accessor and a `project(":core")` literal both arrive as one
// ProjectDependency, so every spelling of a declared dependency is the same check. A dependency added
// later (in `afterEvaluate`, by `withDependencies` or by a substitution rule) is never declared here, so
// every resolved graph is read as well: whatever reaches a classpath is what the rule is about.
gradle.lifecycle.afterProject {
    if (this == rootProject) return@afterProject
    if (!pluginManager.hasPlugin("craftbridge.android.library")) {
        throw GradleException("$path does not apply craftbridge.android.library, so the build's formatting, Lint and Kotlin rules skip it")
    }
    val module = path
    val allowed = if (module == ":core") setOf(module) else setOf(module, ":core")

    fun refuse(other: String): Nothing =
        throw GradleException("$module depends on $other: :core depends on no module of this build, and a plugin module on :core alone")
    for (configuration in configurations) {
        for (dependency in configuration.dependencies.withType(ProjectDependency::class.java)) {
            if (dependency.path !in allowed) refuse(dependency.path)
        }
    }
    configurations.configureEach {
        incoming.afterResolve {
            for (component in resolutionResult.allComponents) {
                val identifier = component.id
                if (identifier is ProjectComponentIdentifier && identifier.projectPath !in allowed) refuse(identifier.projectPath)
            }
        }
    }
}

// A module's build file can also rewire the task graph, so that a check the gate asks for succeeds having
// run nothing: `spotlessCheck` with its dependencies stripped formats no file, and `assembleRelease` with
// its compilation gone checks no Kotlin. So once the graph is ready, each check the gate asks for must
// still bring, in the same project, the tasks that do its work. (A disabled task, or one an `onlyIf`
// skips, stays in the graph; tools/ci.sh fails on its SKIPPED outcome.)
val gateWorkers =
    mapOf(
        "spotlessCheck" to listOf("spotlessKotlin", "spotlessKotlinCheck", "spotlessKotlinGradle", "spotlessKotlinGradleCheck"),
        "lint" to listOf("lintAnalyzeDebug", "lintDebug"),
        // Release-only sources ship in the release AAR, so Lint reads that variant too.
        "lintRelease" to listOf("lintAnalyzeRelease"),
        "assembleRelease" to listOf("compileReleaseKotlin", "bundleReleaseAar"),
        // The tests' Kotlin compiles under the same rules, so a warning in a test fails too.
        "compileDebugUnitTestSources" to listOf("compileDebugUnitTestKotlin"),
        "compileDebugAndroidTestSources" to listOf("compileDebugAndroidTestKotlin"),
        // Each detekt run reads the packages written for its module from its namespace: without that
        // task in the graph no component was checked.
        "detektMain" to listOf("craftbridgeDetektConfig"),
        "detektTest" to listOf("craftbridgeDetektConfig"),
    )
gradle.taskGraph.whenReady {
    val scheduled = allTasks.map { it.path }.toSet()
    for (task in allTasks) {
        val owner = task.path.removeSuffix(task.name)
        val missing = gateWorkers[task.name].orEmpty().map { owner + it }.filterNot { it in scheduled }
        if (missing.isNotEmpty()) {
            throw GradleException("${task.path} no longer brings ${missing.joinToString()}, so it would succeed having checked nothing")
        }
    }
}
