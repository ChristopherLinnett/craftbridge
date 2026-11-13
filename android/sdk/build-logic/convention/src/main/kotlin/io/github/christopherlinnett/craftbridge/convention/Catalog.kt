package io.github.christopherlinnett.craftbridge.convention

import org.gradle.api.Project
import org.gradle.api.artifacts.VersionCatalogsExtension
import org.gradle.kotlin.dsl.getByType

/** The version of [alias] in gradle/libs.versions.toml, the one place a version is written. */
internal fun Project.catalogVersion(alias: String): String =
    extensions
        .getByType<VersionCatalogsExtension>()
        .named("libs")
        .findVersion(alias)
        .map { it.requiredVersion }
        .orElseThrow { IllegalStateException("gradle/libs.versions.toml has no version '$alias'") }
