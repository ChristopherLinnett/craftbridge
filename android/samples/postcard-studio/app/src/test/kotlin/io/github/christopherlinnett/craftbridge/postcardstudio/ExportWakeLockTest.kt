// The wake lock an export holds: taken before it begins and let go of however it ends, since a lock that outlives
// its export keeps the phone's CPU awake for nothing, and an export that begins without one can be put to sleep.

package io.github.christopherlinnett.craftbridge.postcardstudio

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class ExportWakeLockTest {
    /** A lock that keeps the order of what happened to it, and says whether it is held now. */
    private class RecordingLock : ExportLock {
        val events = mutableListOf<String>()
        var held = false

        override fun acquire() {
            events += "acquire"
            held = true
        }

        override fun release() {
            events += "release"
            held = false
        }
    }

    @Test
    fun theLockIsHeldForTheExportAndOnlyForIt() {
        val lock = RecordingLock()
        val answer =
            runBlocking {
                lock.heldDuring {
                    lock.events += "export"
                    assertTrue(lock.held, "the export runs without its lock")
                    "made"
                }
            }
        assertEquals("made", answer)
        assertEquals(listOf("acquire", "export", "release"), lock.events)
        assertFalse(lock.held)
    }

    @Test
    fun theLockIsLetGoOfWhenTheExportFails() {
        val lock = RecordingLock()
        assertFailsWith<IllegalStateException> {
            runBlocking { lock.heldDuring { error("the engine refused") } }
        }
        assertEquals(listOf("acquire", "release"), lock.events)
        assertFalse(lock.held)
    }

    @Test
    fun theLockIsLetGoOfWhenTheExportIsCancelled() {
        val lock = RecordingLock()
        runBlocking {
            val running = CompletableDeferred<Unit>()
            val export =
                launch {
                    lock.heldDuring {
                        running.complete(Unit)
                        awaitCancellation()
                    }
                }
            running.await()
            assertTrue(lock.held, "the export was cancelled before it held the lock")
            export.cancelAndJoin()
        }
        assertEquals(listOf("acquire", "release"), lock.events)
        assertFalse(lock.held)
    }

    @Test
    fun theProgressBarFillsAsFramesAreMade() {
        assertEquals(0f, ExportState.Running(0uL, 0uL).fraction)
        assertEquals(0.25f, ExportState.Running(45uL, 180uL).fraction)
        assertEquals(1f, ExportState.Running(180uL, 180uL).fraction)
    }
}
