// A window the native library holds for an attach is given back however the attach ends.

package io.github.christopherlinnett.craftbridge.core

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class AttachingWindowTest {
    @Test
    fun theWindowIsGivenBackAfterAnAttachThatTookIt() {
        val released = mutableListOf<Long>()
        val attached = mutableListOf<Long>()
        attachingWindow(7L, { released += it }) { attached += it }
        assertEquals(listOf(7L), attached)
        assertEquals(listOf(7L), released)
    }

    @Test
    fun theWindowIsGivenBackAfterAnAttachThatFailedAndTheFailureGoesOn() {
        val released = mutableListOf<Long>()
        val refused =
            assertFailsWith<IllegalStateException> {
                attachingWindow(8L, { released += it }) { error("the surface cannot become a preview") }
            }
        assertEquals("the surface cannot become a preview", refused.message)
        assertEquals(listOf(8L), released)
    }
}
