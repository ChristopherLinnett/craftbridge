package io.github.christopherlinnett.craftbridge.convention

import org.gradle.api.GradleException
import org.gradle.api.Project
import java.io.File

/**
 * The repository's root folder, where the files every platform shares sit: the one `.editorconfig`, and the
 * conformance data in `conformance/`. It is the nearest folder above the build that holds the `.editorconfig`,
 * which is the root's alone (the gate refuses every other), so the build at `android/sdk` and a sample's own
 * build at `android/samples/<app>` find the same folder from where each sits.
 */
internal val Project.repositoryRoot: File
    get() =
        generateSequence(rootDir) { it.parentFile }.firstOrNull { it.resolve(".editorconfig").isFile }
            ?: throw GradleException("no .editorconfig above $rootDir: the repository root holds the one every Gradle build reads")

/** The build-logic every build of the repository takes its convention plugins and detekt rules from. */
internal val Project.buildLogicFolder: File
    get() = repositoryRoot.resolve("android/sdk/build-logic")
