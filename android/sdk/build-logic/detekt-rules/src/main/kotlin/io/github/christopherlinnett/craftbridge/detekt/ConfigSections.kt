package io.github.christopherlinnett.craftbridge.detekt

import dev.detekt.api.Config

/**
 * Each section of this config, by name: a key whose value is itself a map, which detekt reads as a rule
 * set at the top of a run's config and as one rule inside a rule set. The keys beside the sections are
 * plain settings, so a walk over a run's rule sets and rules descends into the sections alone.
 */
internal fun Config.sections(): List<Pair<String, Config>> =
    subConfigKeys()
        .filter { valueOrNull<Any>(it) is Map<*, *> }
        .map { it to subConfig(it) }
