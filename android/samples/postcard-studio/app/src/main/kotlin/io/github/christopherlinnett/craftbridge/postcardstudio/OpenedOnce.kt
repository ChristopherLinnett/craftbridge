package io.github.christopherlinnett.craftbridge.postcardstudio

import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import java.io.File
import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds

/** How long the end of an engine whose card did not open may take, since the next try waits for it. */
val SHUT_DOWN_TIMEOUT: Duration = 10.seconds

/** An engine of type [E] and the card of type [C] opened in it. */
class Opened<E, C>(
    val engine: E,
    val card: C,
)

/**
 * An engine and its card, opened by the first try that succeeds and kept from then on, so the app runs one engine.
 * Each try first copies the [template] out of the app, so a copy that fails starts no engine, then [start]s an
 * engine and [open]s the template in it; an engine whose card does not open, because the open failed or the try was
 * cancelled, is [shutDown] before the try ends, so no failed try leaves an engine running for the next to add to.
 * That end cannot change how the try ends, nor hold the next try back: what it throws, or its running out of
 * [shutDownTimeout], goes to [onEndFailure] alone, and an exception that reporter throws in turn is dropped, since the
 * try's own failure or cancellation is the answer the caller needs. An `Error` is the runtime's own and goes on, as it
 * does everywhere; the next try is not held back by it either. Only a kept engine is ever [engine], so no caller
 * reaches one that a failed try is ending.
 */
class OpenedOnce<E, C>(
    private val template: () -> File,
    private val start: suspend () -> E,
    private val open: suspend (E, File) -> C,
    private val shutDown: suspend (E) -> Unit,
    private val onEndFailure: (E, Exception) -> Unit,
    private val shutDownTimeout: Duration = SHUT_DOWN_TIMEOUT,
) {
    private val opening = Mutex()

    // Read by callers that take no turn at the lock, such as the one that lets a preview go.
    @Volatile
    private var opened: Opened<E, C>? = null

    /** The engine, when one is open. */
    val engine: E? get() = opened?.engine

    /** The engine and its card, opened now when no try has opened them yet. */
    suspend fun get(): Opened<E, C> = opening.withLock { opened ?: openFresh().also { opened = it } }

    private suspend fun openFresh(): Opened<E, C> {
        val file = template()
        val engine = start()
        var kept = false
        try {
            val card = open(engine, file)
            kept = true
            return Opened(engine, card)
        } finally {
            // A cancelled try must still end its engine, which a cancelled coroutine's own calls would not.
            if (!kept) withContext(NonCancellable) { endQuietly(engine) }
        }
    }

    /** Ends [engine] within the time allowed, telling [onEndFailure] of a failure or of the time running out. */
    private suspend fun endQuietly(engine: E) {
        try {
            withTimeout(shutDownTimeout) { shutDown(engine) }
        } catch (failure: Exception) {
            // The engine is lost either way: the thread that holds it is the engine's own, and there is nothing more to do.
            try {
                onEndFailure(engine, failure)
            } catch (ignored: Exception) {
                // A reporter that fails has nobody to tell, and must not replace the try's own failure or cancellation.
            }
        }
    }
}
