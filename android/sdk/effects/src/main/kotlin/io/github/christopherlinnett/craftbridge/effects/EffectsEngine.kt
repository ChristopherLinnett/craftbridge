// The entry point of the EffectCraft API. The generated command namespaces in `generated/` hang off it.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.JSONValue

/** An EffectCraft engine: the typed command namespaces over its one generic request call. */
public class EffectsEngine
    internal constructor(
        private val transport: suspend (method: String, params: JSONValue) -> JSONValue,
    ) {
        /** Sends one request of the engine's method table, such as `"exec"`, and returns its reply. Internal:
         * the public calls are the typed ones built on it. */
        internal suspend fun request(
            method: String,
            params: JSONValue,
        ): JSONValue = transport(method, params)

        /** Runs the command `id`. `extra` holds keys the type rules left untyped, none when it is `null`, and
         * `params` the keys the caller typed, a `null` value for one left out; a typed key replaces an extra
         * one, so a typed value is not lost to a stray key. The generated facade passes its keys as pairs,
         * so its own code calls no library function a command or key of the same name could take. */
        internal suspend fun command(
            id: String,
            extra: Map<String, JSONValue>?,
            vararg params: Pair<String, JSONValue?>,
        ): JSONValue {
            val keys = extra.orEmpty().toMutableMap()
            for ((key, value) in params) {
                if (value != null) {
                    keys[key] = value
                }
            }
            return request("exec", JSONValue.Object(mapOf("command" to JSONValue.Text(id), "params" to JSONValue.Object(keys))))
        }
    }
