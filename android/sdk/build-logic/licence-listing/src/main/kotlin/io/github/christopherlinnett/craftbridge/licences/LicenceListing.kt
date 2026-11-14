package io.github.christopherlinnett.craftbridge.licences

import org.gradle.api.artifacts.Configuration
import org.gradle.api.artifacts.component.ModuleComponentIdentifier
import java.util.SortedSet

/**
 * Adds every external module resolved onto [classpaths] to [modules], each as one `group:name`,
 * which is the line shape `cargo xtask rules` matches against `tools/licences.toml`. Resolution is
 * read when the calling task runs, exactly as Gradle's own `dependencies` report reads it, so the
 * answer names only modules that really land on a classpath. AGENTS §8 holds every dependency to
 * the allowlist, what ships and what the gate runs alike, so both prints go through this one
 * definition; its own build's reason for sitting here is in that folder's build file.
 */
object LicenceListing {
    fun addResolvedModules(
        classpaths: Iterable<Configuration>,
        modules: SortedSet<String>,
    ) {
        for (classpath in classpaths) {
            classpath.incoming.resolutionResult.allComponents.forEach { component ->
                val identifier = component.id
                if (identifier is ModuleComponentIdentifier) {
                    modules += "${identifier.group}:${identifier.module}"
                }
            }
        }
    }
}
