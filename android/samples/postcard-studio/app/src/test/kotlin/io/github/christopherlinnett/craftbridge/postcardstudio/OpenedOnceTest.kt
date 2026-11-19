// The app's one engine: a try that fails, before the engine starts or after, leaves no engine running, and the try
// that then succeeds starts the one engine the app keeps, since every engine left behind holds a thread of its own.

package io.github.christopherlinnett.craftbridge.postcardstudio

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.yield
import java.io.File
import java.io.IOException
import kotlin.concurrent.thread
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertIs
import kotlin.test.assertSame
import kotlin.time.Duration
import kotlin.time.Duration.Companion.milliseconds

class OpenedOnceTest {
    /** Engines a test started and ended, by number, and how its next copy and open behave. */
    private class Engines {
        var started = 0
        val ended = mutableListOf<Int>()
        var copyFails = false
        var openFails = false
        var openWaits = false
        var shutDownFails = false
        var shutDownHangs = false
        var reporterFails = false
        var reporterErrs = false

        /** Completed when a try reaches its open; a test that runs a second try gives it a new one to wait on. */
        var opening = CompletableDeferred<Unit>()

        /** What the end of each engine reported, and which engine a caller could reach while one was ending. */
        val endFailures = mutableListOf<Pair<Int, Exception>>()
        val reachableWhileEnding = mutableListOf<Int?>()
        lateinit var current: OpenedOnce<Int, String>

        val running: Int get() = started - ended.size

        fun opened(shutDownTimeout: Duration = SHUT_DOWN_TIMEOUT) =
            OpenedOnce(
                template = {
                    if (copyFails) throw IOException("the disk is full")
                    File("Birthday Card.ectemplate")
                },
                // Starting a real engine suspends, so callers at once meet halfway through it.
                start = {
                    yield()
                    ++started
                },
                open = { engine: Int, _: File ->
                    opening.complete(Unit)
                    if (openWaits) awaitCancellation()
                    check(!openFails) { "not an EffectCraft template" }
                    "card of engine $engine"
                },
                // Ending a real engine suspends, which a cancelled coroutine's own calls refuse to do.
                shutDown = { engine: Int ->
                    reachableWhileEnding += current.engine
                    yield()
                    if (shutDownHangs) awaitCancellation()
                    check(!shutDownFails) { "the engine would not end" }
                    ended += engine
                },
                onEndFailure = { engine: Int, failure: Exception ->
                    endFailures += engine to failure
                    check(!reporterFails) { "the reporter failed too" }
                    if (reporterErrs) throw AssertionError("the reporter's runtime gave out")
                },
                shutDownTimeout = shutDownTimeout,
            ).also { current = it }
    }

    @Test
    fun aCopyThatFailsStartsNoEngineAndTheTryAfterItStartsOne() {
        val engines = Engines().apply { copyFails = true }
        val opened = engines.opened()
        runBlocking {
            repeat(3) { assertFailsWith<IOException> { opened.get() } }
            assertEquals(0, engines.started, "a copy that failed started an engine")
            engines.copyFails = false
            val first = opened.get()
            assertSame(first, opened.get(), "a second call opened again")
        }
        assertEquals(1, engines.started)
        assertEquals(1, engines.running)
    }

    @Test
    fun aCardThatDoesNotOpenEndsItsEngineAndTheTryAfterItKeepsOne() {
        val engines = Engines().apply { openFails = true }
        val opened = engines.opened()
        runBlocking {
            repeat(2) { assertFailsWith<IllegalStateException> { opened.get() } }
            engines.openFails = false
            assertEquals("card of engine 3", opened.get().card)
            opened.get()
        }
        assertEquals(listOf(1, 2), engines.ended, "an engine whose card did not open was left running")
        assertEquals(1, engines.running)
        assertEquals(3, opened.engine)
    }

    @Test
    fun aCancelledTryEndsTheEngineItStarted() {
        val engines = Engines().apply { openWaits = true }
        val opened = engines.opened()
        runBlocking {
            val attempt = launch { opened.get() }
            engines.opening.await()
            attempt.cancelAndJoin()
        }
        assertEquals(listOf(1), engines.ended, "a cancelled try left its engine running")
        assertEquals(0, engines.running)
        assertEquals(null, opened.engine)
    }

    @Test
    fun callersAtOnceShareTheOneEngine() {
        val engines = Engines()
        val opened = engines.opened()
        val cards = runBlocking { List(4) { async { opened.get() } }.map { it.await().card } }
        assertEquals(List(4) { "card of engine 1" }, cards)
        assertEquals(1, engines.started)
    }

    @Test
    fun anEngineThatWillNotEndChangesNeitherTheFailureNorTheCancellationOfItsTry() {
        val engines =
            Engines().apply {
                openFails = true
                shutDownFails = true
            }
        val opened = engines.opened()
        runBlocking {
            val failure = assertFailsWith<IllegalStateException> { opened.get() }
            assertEquals("not an EffectCraft template", failure.message, "the end of the engine replaced the try's own failure")
            engines.openFails = false
            engines.openWaits = true
            engines.opening = CompletableDeferred()
            val attempt = launch { opened.get() }
            engines.opening.await()
            attempt.cancelAndJoin()
            assertEquals(true, attempt.isCancelled)
        }
        assertEquals(listOf(1, 2), engines.endFailures.map { it.first }, "an engine that would not end went unreported")
        assertEquals(listOf("the engine would not end", "the engine would not end"), engines.endFailures.map { it.second.message })
    }

    @Test
    fun anEngineThatHangsOnEndingDoesNotHoldTheNextTryBack() {
        val engines =
            Engines().apply {
                openFails = true
                shutDownHangs = true
            }
        val opened = engines.opened(shutDownTimeout = 50.milliseconds)
        var card: String? = null
        // A try that is held back never returns, so the test runs it on a thread of its own and waits a while for it.
        val attempt =
            thread(isDaemon = true) {
                runBlocking {
                    assertFailsWith<IllegalStateException> { opened.get() }
                    engines.openFails = false
                    engines.shutDownHangs = false
                    card = opened.get().card
                }
            }
        attempt.join(10_000)
        assertEquals(false, attempt.isAlive, "an engine that hangs on ending held the next try back")
        assertEquals("card of engine 2", card)
        assertEquals(1, engines.endFailures.single().first, "an engine that hung on ending went unreported")
        assertIs<TimeoutCancellationException>(engines.endFailures.single().second)
    }

    /** The engine a caller reaches, the preview's end among them, is only ever the kept one, never one being ended. */
    @Test
    fun noCallerReachesAnEngineThatAFailedTryIsEnding() {
        val engines = Engines().apply { openFails = true }
        val opened = engines.opened()
        runBlocking {
            assertFailsWith<IllegalStateException> { opened.get() }
            engines.openFails = false
            engines.openWaits = true
            engines.opening = CompletableDeferred()
            val attempt = launch { opened.get() }
            engines.opening.await()
            attempt.cancelAndJoin()
        }
        assertEquals(listOf<Int?>(null, null), engines.reachableWhileEnding)
        assertEquals(null, opened.engine)
    }

    @Test
    fun aReporterThatFailsChangesNeitherTheFailureOfItsTryNorTheTryAfterIt() {
        val engines =
            Engines().apply {
                openFails = true
                shutDownFails = true
                reporterFails = true
            }
        val opened = engines.opened()
        runBlocking {
            val failure = assertFailsWith<IllegalStateException> { opened.get() }
            assertEquals("not an EffectCraft template", failure.message, "the reporter's failure replaced the try's own")
            engines.openFails = false
            engines.shutDownFails = false
            assertEquals("card of engine 2", opened.get().card)
        }
    }

    /** An `Error` the reporter throws is the runtime's own and goes on, but the lock is let go and nothing is kept. */
    @Test
    fun aReporterErrorEndsItsTryButLeavesTheTryAfterItFree() {
        val engines =
            Engines().apply {
                openFails = true
                shutDownFails = true
                reporterErrs = true
            }
        val opened = engines.opened()
        runBlocking {
            assertFailsWith<AssertionError> { opened.get() }
            assertEquals(null, opened.engine)
            engines.openFails = false
            engines.shutDownFails = false
            engines.reporterErrs = false
            assertEquals("card of engine 2", opened.get().card)
        }
    }
}
