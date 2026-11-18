// A cancellation reaches the engine as its stop while the call still runs, and the caller is told the call's own answer.

package io.github.christopherlinnett.craftbridge.core

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineExceptionHandler
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * [stopWhenCancelled] is all of what cancelling a call over the boundary depends on: the engine cannot
 * be cut short from here, only asked to stop, and only while it is still running. An engine is only on a
 * device, so these tests hand the rule work that behaves as an export does and watch what the caller is
 * told.
 */
class StopWhenCancelledTest {
    /**
     * Work shaped like an export: it looks for the stop at every beat and, once it sees it, answers that
     * it was cancelled. It gives up at a ceiling, so a stop that never arrives, or arrives only once the
     * work has ended, fails a test instead of holding it.
     */
    private class Export {
        var stopped = false
        var beats = 0

        fun stop() {
            stopped = true
        }

        suspend fun run(): String {
            while (!stopped && beats < CEILING) {
                beats += 1
                delay(BEAT_MILLIS)
            }
            if (stopped) {
                throw CancelledException("cancelled")
            }
            return "rendered"
        }
    }

    @Test
    fun aCancellationSetsTheStopWhileTheCallRunsAndTheCallerIsToldTheCallStopped(): Unit =
        bounded {
            val export = Export()
            val answered = answerOfACallCancelledWhileItRuns(export, cancels = 1, stop = export::stop)
            assertTrue(export.beats < CEILING, "the work ran to its ceiling: the stop arrived too late for it, if at all")
            assertEndedAsTheCancelOf(answered)
        }

    @Test
    fun aCallerCancelledBeforeItMakesTheCallStopsTheWorkBeforeItsFirstBeat(): Unit =
        bounded {
            val export = Export()
            var answered: Throwable? = null
            launch {
                // Cancelled from inside, so the cancel has landed before the call is made, as one from
                // another thread can land at any moment before it.
                cancel()
                answered = runCatching { stopWhenCancelled(export::stop) { export.run() } }.exceptionOrNull()
            }.join()
            assertEquals(0, export.beats, "the work ran ${export.beats} beats for a caller that was cancelled before it asked")
            assertEndedAsTheCancelOf(answered)
        }

    @Test
    fun aCallItsCallerCancelledFailsNothingAroundIt(): Unit =
        bounded {
            val export = Export()
            val running = CompletableDeferred<Unit>()
            var uncaught: Throwable? = null
            // A scope shaped like an app's own, whose children fail alone, and whose handler is where a
            // child's failure goes: on Android, with no handler, it is the end of the app. It has no
            // parent, as an app's scope has none, so nothing here waits for it to end.
            val call =
                launch(SupervisorJob() + CoroutineExceptionHandler { _, failure -> uncaught = failure }) {
                    stopWhenCancelled(export::stop) {
                        running.complete(Unit)
                        export.run()
                    }
                }
            running.await()
            call.cancel()
            call.join()
            assertNull(uncaught, "a cancel the caller made reached its scope as a failure")
        }

    @Test
    fun aSecondCancelOfTheCallerChangesNothing(): Unit =
        bounded {
            val export = Export()
            var stops = 0
            val answered =
                answerOfACallCancelledWhileItRuns(export, cancels = 2) {
                    stops += 1
                    export.stop()
                }
            assertEquals(1, stops, "a cancel made twice stopped the work $stops times")
            assertEndedAsTheCancelOf(answered)
        }

    @Test
    fun aStopTheCallerDidNotMakeIsAnsweredAsTheEngineAnsweredIt(): Unit =
        bounded {
            val export = Export()
            val answered =
                runCatching {
                    stopWhenCancelled(stop = {}) {
                        // As the engine stops when it is shut down, with nobody cancelling the caller.
                        export.stop()
                        export.run()
                    }
                }.exceptionOrNull()
            assertIs<CancelledException>(answered, "a caller that cancelled nothing was told its own cancel")
        }

    @Test
    fun aCallThatAnswersHandsItsAnswerBack(): Unit =
        bounded {
            assertEquals("rendered", stopWhenCancelled(stop = {}) { "rendered" })
        }

    /**
     * Run a test's body with a bound on it, so a call that something of its own still holds open fails
     * the test rather than hangs it. The body is a `Unit`, because TestNG passes over a test method that
     * returns a value, without failing it.
     */
    private fun bounded(body: suspend CoroutineScope.() -> Unit) {
        runBlocking { withTimeout(HOLD_MILLIS, body) }
    }

    /**
     * How a call of `export`, made in a coroutine of its own with `stop` as its stop, ended once that
     * coroutine was cancelled `cancels` times while the work ran, as the coroutine itself saw it.
     */
    private suspend fun CoroutineScope.answerOfACallCancelledWhileItRuns(
        export: Export,
        cancels: Int,
        stop: () -> Unit,
    ): Throwable? {
        val running = CompletableDeferred<Unit>()
        var answered: Throwable? = null
        val call =
            launch {
                answered =
                    runCatching {
                        stopWhenCancelled(stop) {
                            running.complete(Unit)
                            export.run()
                        }
                    }.exceptionOrNull()
            }
        running.await()
        repeat(cancels) { call.cancel() }
        call.join()
        return answered
    }

    /** That `answered` is how a cancelled coroutine ends, carrying what the engine answered once it stopped. */
    private fun assertEndedAsTheCancelOf(answered: Throwable?) {
        assertIs<CancellationException>(answered, "the call did not end as the cancel it was: $answered")
        assertIs<CancelledException>(answered.cause, "the engine's answer to the stop was not waited for, or was lost")
    }

    private companion object {
        /** The beats the work runs at most, far more than a stop that arrives in time lets it run. */
        const val CEILING = 100
        const val BEAT_MILLIS = 5L

        /** Far longer than any of these calls takes, the work's whole ceiling included. */
        const val HOLD_MILLIS = 5_000L
    }
}
