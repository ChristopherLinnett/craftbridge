package io.github.christopherlinnett.craftbridge.convention

import org.gradle.api.Project
import org.gradle.api.artifacts.MinimalExternalModuleDependency
import org.gradle.api.artifacts.VersionCatalog
import org.gradle.api.artifacts.VersionCatalogsExtension
import org.gradle.api.provider.Provider
import org.gradle.kotlin.dsl.getByType

private val Project.catalog: VersionCatalog
    get() = extensions.getByType<VersionCatalogsExtension>().named("libs")

/** The version of [alias] in gradle/libs.versions.toml, the one place a version is written. */
internal fun Project.catalogVersion(alias: String): String =
    catalog
        .findVersion(alias)
        .map { it.requiredVersion }
        .orElseThrow { IllegalStateException("gradle/libs.versions.toml has no version '$alias'") }

/** The library [alias] in gradle/libs.versions.toml, coordinate and version both, as the catalog writes them once. */
internal fun Project.catalogLibrary(alias: String): Provider<MinimalExternalModuleDependency> =
    catalog
        .findLibrary(alias)
        .orElseThrow { IllegalStateException("gradle/libs.versions.toml has no library '$alias'") }
