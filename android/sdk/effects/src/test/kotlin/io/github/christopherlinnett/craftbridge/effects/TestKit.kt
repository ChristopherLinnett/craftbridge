// What every test in this module shares: a transport that records the requests, answers scripted to
// the method asked, and a way to await a call that never really suspends.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.Engine
import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.ffi.core.NoHandle
import kotlin.coroutines.Continuation
import kotlin.coroutines.CoroutineContext
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.coroutines.startCoroutine
import io.github.christopherlinnett.craftbridge.ffi.core.Engine as BoundaryEngine

/** A core engine that answers nothing: the fake transports stand in for the whole engine, and no
 * host test reaches the core itself. It stands up without the native library. */
internal fun fakeCore(): Engine = Engine.fromBoundary(BoundaryEngine(NoHandle))

/** What the engine was asked, kept for the test to read; each method answers what the test scripted
 * for it, or the same plain reply when nothing was scripted. */
internal class Asked(
    private val scripted: Map<String, JSONValue> = emptyMap(),
) {
    val all: MutableList<Pair<String, JSONValue>> = mutableListOf()

    fun record(
        method: String,
        params: JSONValue,
    ): JSONValue {
        all.add(method to params)
        return scripted[method] ?: JSONValue.Text("done")
    }
}

/** The engine the tests call: a fake core and a recording transport. */
internal fun fakeEngine(asked: Asked): EffectsEngine = EffectsEngine(fakeCore()) { method, params -> asked.record(method, params) }

/** The envelope the transport carries every command in: the command id and its params, in one Object. */
internal fun exec(
    command: String,
    params: JSONValue,
) = JSONValue.Object(
    mapOf(
        "command" to JSONValue.Text(command),
        "params" to params,
    ),
)

/** Awaiting a call that cannot suspend: every fake engine answers on the spot, so the call has
 * finished before `startCoroutine` returns and the outcome is already here — no runner beyond the
 * coroutine machinery the standard library itself carries. */
internal fun <R> await(call: suspend () -> R): Result<R> {
    var outcome: Result<R>? = null
    call.startCoroutine(
        object : Continuation<R> {
            override val context: CoroutineContext = EmptyCoroutineContext

            override fun resumeWith(result: Result<R>) {
                outcome = result
            }
        },
    )
    return outcome ?: Result.failure(IllegalStateException("the call did not finish on the spot"))
}
