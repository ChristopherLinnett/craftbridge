package io.github.christopherlinnett.craftbridge.convention

import org.gradle.api.Project
import java.io.File

/**
 * The repository's root folder, where the files every platform shares sit: the one `.editorconfig`, and the
 * conformance data in `conformance/`. The Gradle build sits at `android/sdk`, two folders below it.
 */
internal val Project.repositoryRoot: File
    get() = rootDir.resolve("../..")
