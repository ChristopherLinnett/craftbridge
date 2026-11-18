// One step of the `run` call's list of commands.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.JSONValue

/** One step of a [EffectsEngine.run]: the command's id and the parameters it takes. A plain class, not a
 * `data class`: a data class's generated `copy` and `componentN` are public API a later field would
 * break, which the AndroidX API guidelines keep out of a library. */
public class RunStep(
    public val command: String,
    public val params: Map<String, JSONValue> = emptyMap(),
)
