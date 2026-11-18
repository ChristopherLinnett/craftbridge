// What every test in this module shares: a transport that records the requests, answers scripted to
// the method asked, and a way to await a call that never really suspends.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.Engine
import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.ffi.core.FrameBytes
import io.github.christopherlinnett.craftbridge.ffi.core.NoHandle
import io.github.christopherlinnett.craftbridge.ffi.core.PixelFormat
import kotlin.coroutines.Continuation
import kotlin.coroutines.CoroutineContext
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.coroutines.startCoroutine
import io.github.christopherlinnett.craftbridge.ffi.core.Engine as BoundaryEngine

/** A core engine that answers nothing: the fake transports stand in for the whole engine, and no
 * host test reaches the core itself. It stands up without the native library. */
internal fun fakeCore(): Engine = Engine.fromBoundary(BoundaryEngine(NoHandle))

/** A boundary engine that records the frame and preview requests the core's calls hand it, and answers a
 * frame no picture can be made of, so a test reads the request and needs no Android to convert one. Its
 * report of a held preview surface and its count of detachments are what the preview calls of the typed
 * facade answer through, so a test can read the one call each of them is. */
internal class RecordingBoundary : BoundaryEngine(NoHandle) {
    val frames: MutableList<Pair<String, PixelFormat>> = mutableListOf()
    val presents: MutableList<String> = mutableListOf()
    var detachments = 0
    var holding = false

    override suspend fun frame(
        params: String,
        format: PixelFormat,
    ): FrameBytes {
        frames.add(params to format)
        return FrameBytes(1u, 1u, format, ByteArray(0))
    }

    override suspend fun present(params: String) {
        presents.add(params)
    }

    override suspend fun detachPreview() {
        detachments++
    }

    override suspend fun hasPreviewSurface(): Boolean = holding
}

/** The engine over a recording boundary: what its typed picture and preview calls ask of the core. */
internal fun boundaryEngine(boundary: RecordingBoundary): EffectsEngine = EffectsEngine(Engine.fromBoundary(boundary))

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
