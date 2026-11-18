// Every report the engine makes reaches the lambda the call was given.

package io.github.christopherlinnett.craftbridge.core

import kotlin.test.Test
import kotlin.test.assertEquals

class ProgressTest {
    @Test
    fun everyReportReachesTheHandlerOfTheCallThatAskedForIt() {
        val heard = mutableListOf<Triple<ULong, ULong, Double?>>()
        val listener = ProgressOf { done, total, secondsLeft -> heard += Triple(done, total, secondsLeft) }
        listener.progress(0uL, 12uL, null)
        listener.progress(9uL, 12uL, 1.5)
        assertEquals(listOf(Triple(0uL, 12uL, null as Double?), Triple(9uL, 12uL, 1.5)), heard)
    }
}
