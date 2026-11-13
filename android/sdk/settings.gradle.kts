import org.gradle.api.artifacts.ProjectDependency
import org.gradle.api.artifacts.component.ProjectComponentIdentifier

pluginManagement {
    includeBuild("build-logic")
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

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
        "assembleRelease" to listOf("compileReleaseKotlin", "bundleReleaseAar"),
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
