package io.github.christopherlinnett.craftbridge.convention

import org.gradle.api.Project
import org.gradle.api.services.BuildService
import org.gradle.api.services.BuildServiceParameters

/**
 * A project's build folder, which one of the project's tasks holds at a time. When a task's sources
 * are empty, Gradle skips it and deletes the outputs it recorded the last time it ran, then each
 * folder those deletions left empty, up to the build folder: a check that the folder is empty, then a
 * separate delete. Gradle keeps two running tasks apart only where their declared outputs nest, and
 * AGP gives sibling tasks outputs that do not nest under folders they share
 * (`build/intermediates/built_in_kotlinc`, `build/intermediates/java_res_compressed_jar`,
 * `build/kotlin`). The configuration cache runs a project's tasks in parallel, which `--no-parallel`
 * does not switch off, so when a module loses its last sources two of its tasks skipped at once can
 * each find a shared folder empty, and the slower fails on the folder the other has just deleted.
 * A build service that one task may use at a time is how Gradle coordinates tasks over a resource its
 * plan cannot see: a task holds it from its start to its end, skip and cleanup included, so a
 * project's tasks take turns while other projects, and the included build, still run beside them.
 */
internal abstract class BuildFolderTurn : BuildService<BuildServiceParameters.None>

/** Gives every task of the project its turn at the project's build folder ([BuildFolderTurn]). */
internal fun Project.takeTurnsAtBuildFolder() {
    val turn =
        gradle.sharedServices.registerIfAbsent("craftbridgeBuildFolder$path", BuildFolderTurn::class.java) {
            maxParallelUsages.set(1)
        }
    tasks.configureEach { usesService(turn) }
}
