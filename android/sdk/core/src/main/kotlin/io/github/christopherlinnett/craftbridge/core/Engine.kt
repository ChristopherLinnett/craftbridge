// The engine's calls as coroutines: progress to a lambda, and a cancel the engine obeys.

package io.github.christopherlinnett.craftbridge.core

import io.github.christopherlinnett.craftbridge.ffi.core.setDeviceCodecs
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import io.github.christopherlinnett.craftbridge.ffi.core.CancelToken as BoundaryCancelToken
import io.github.christopherlinnett.craftbridge.ffi.core.Engine as BoundaryEngine
import io.github.christopherlinnett.craftbridge.ffi.core.Folders as BoundaryFolders

/**
 * One engine, called as Kotlin coroutines: [request] for the calls that answer at once,
 * [requestWithProgress] for the long ones that report how far they have got, [shutDown] to end it.
 *
 * Every call fails with a [CraftException] and with nothing else, so an app handles a refusal by
 * catching one of the plan's rows and never by guessing what a foreign exception type holds
 * (AGENTS.md §3). Every call can be cancelled as a coroutine, and ends as a cancelled coroutine does,
 * with a `CancellationException`; what the cancel stops differs: a [requestWithProgress] is stopped in
 * the engine itself, as the app would stop it with its own hand, while a [request] the engine has taken
 * runs to its end there and only the wait for its answer stops.
 *
 * What is not here is what an app cannot name yet: the frame and the preview belong with the surfaces
 * that hold them, and `mcp`, `suspendGpu` and `resumeGpu` with the plugin and the background policy
 * that call them, so each comes with the issue that gives it somewhere to be used.
 */
public class Engine internal constructor(
    internal val boundary: BoundaryEngine,
) {
    public companion object {
        /**
         * The engine a Craft app plugin started through its own native boundary, as the core's calls
         * take it. A plugin's own start goes through [start], which calls this once it has handed the
         * device's codec list over: an engine started without the list decodes every stream in software.
         * An app is handed an engine and names no boundary.
         */
        public fun fromBoundary(boundary: BoundaryEngine): Engine = Engine(boundary)

        /**
         * Start an engine in [folders] through a Craft app plugin's own native start, [begin], which is handed the
         * folders as the boundary takes them. The device's codec list is read and handed over first, since the
         * start registers the hardware decoders from it: an engine started without it decodes every stream in
         * software, and its `info` says `hardwareDecoding` is false. A plugin's own start calls this; an app is
         * handed an engine and names no boundary.
         *
         * @throws CraftException for whatever [begin] refuses the start with.
         */
        public fun start(
            folders: Folders,
            begin: (BoundaryFolders) -> BoundaryEngine,
        ): Engine {
            setDeviceCodecs(deviceCodecs())
            return fromBoundary(mapped { begin(folders.toFfi()) })
        }
    }

    /**
     * Ask the engine for one method. Its parameters are [params] as JSON text, `{}` when it takes none;
     * the reply is the engine's JSON text.
     *
     * @throws CraftException for whatever the engine answers the call with.
     */
    public suspend fun request(
        method: String,
        params: String = "{}",
    ): String = mapped { boundary.request(method, params) }

    /**
     * Ask the engine for a method that reports progress: `export` is one, and methods that report
     * progress are the only ones the engine reports anything during.
     *
     * Cancelling the coroutine this runs in, before this call or during it, sets the engine's own cancel
     * token, so the work stops at its next beat, or before its first, instead of running on with nobody
     * listening. The call still waits for the engine to answer, because the engine's wind-down is what
     * deletes the files the export had written; the engine stops at its next beat, so the wait is about
     * one beat and not the rest of the export. Then the call ends as a cancelled coroutine does, with a
     * `CancellationException`, whose cause is what the engine answered — [CancelledException] once it
     * stopped — so the scope it runs in reads it as the cancel it is and not as a failure.
     *
     * @param method the method to ask for, one the app answers with progress.
     * @param params its parameters as JSON text, `{}` when it takes none.
     * @param onProgress every report the engine makes, as it makes it.
     * @throws CraftException for anything the engine answers it with, [CancelledException] when
     * something other than this coroutine stopped it, such as [shutDown].
     */
    public suspend fun requestWithProgress(
        method: String,
        params: String = "{}",
        onProgress: ProgressHandler,
    ): String {
        val cancel = BoundaryCancelToken()
        val progress = ProgressOf(onProgress)
        return stopWhenCancelled(stop = { cancel.cancel() }) {
            mapped { boundary.requestWithProgress(method, params, progress, cancel) }
        }
    }

    /**
     * End the engine: its thread finishes, and every later call of this engine answers that it can no
     * longer be answered. An app calls it when it is done with the engine.
     *
     * @throws CraftException if the engine could not be ended.
     */
    public suspend fun shutDown() {
        mapped { boundary.shutDown() }
    }
}

/**
 * Run [call], work the engine does that cannot be cut short from here, and answer what it answers,
 * with a cancellation of the calling coroutine handed to the engine as [stop], the engine's own cancel.
 *
 * The stop has to reach the engine *while the call runs*, so it is set by a child of the call, which
 * begins cancelling as soon as the caller does; waiting for the call to finish would set it only after
 * the work it was meant to stop. The child starts before the call is made, in the caller's own frame
 * rather than through its dispatcher: a child cancelled before its dispatcher starts it never runs at
 * all, so a caller cancelled before the call, or in the moment after, would leave the work to run to its
 * end with nothing to stop it. Once the call has answered, the child is ended too, setting a stop no
 * call reads any more.
 *
 * [call]'s answer is waited for even after the caller was cancelled: the engine winds the work down,
 * and its wind-down is what deletes the files it had written. A caller that was cancelled is then
 * told so as a coroutine is, with a [CancellationException] whose cause is what the engine answered:
 * a coroutine that ends a cancel by throwing anything else fails the scope it runs in, which at the
 * top of an app's scope is an uncaught exception, and on Android that ends the app.
 */
internal suspend fun <R> stopWhenCancelled(
    stop: () -> Unit,
    call: suspend () -> R,
): R =
    try {
        coroutineScope {
            val stopWatching =
                launch(start = CoroutineStart.UNDISPATCHED) {
                    try {
                        awaitCancellation()
                    } catch (cancelled: CancellationException) {
                        stop()
                    }
                }
            try {
                withContext(NonCancellable) { call() }
            } finally {
                // The scope waits for its children, so a child left watching would hold the call open
                // after it had answered.
                stopWatching.cancel()
            }
        }
    } catch (answered: CraftException) {
        // Outside the scope above, which would keep the cancel it was ended with and drop this cause.
        if (!currentCoroutineContext().isActive) {
            throw CancellationException(answered.message, answered)
        }
        throw answered
    }
