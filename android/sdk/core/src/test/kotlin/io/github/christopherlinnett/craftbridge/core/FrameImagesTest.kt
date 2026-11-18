// The straight frame the core answers becomes the colours Android's `setPixels` reads.

package io.github.christopherlinnett.craftbridge.core

import kotlin.test.Test
import kotlin.test.assertEquals

class FrameImagesTest {
    @Test
    fun everyPixelBecomesTheColourIntWithItsAlphaOnTop() {
        val rgba = byteArrayOf(0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x08)
        val colours = IntArray(2)
        straightRgbaToArgb(rgba, 0, 2, colours)
        assertEquals(listOf(0x44112233, 0x08556677), colours.toList())
    }

    @Test
    fun aByteAboveSevenBitsIsReadUnsignedAndNeverSpillsIntoItsNeighbours() {
        val rgba = byteArrayOf(0x80.toByte(), 0x90.toByte(), 0xA0.toByte(), 0x10)
        val colours = IntArray(1)
        straightRgbaToArgb(rgba, 0, 1, colours)
        assertEquals(0x108090A0, colours.single())
    }

    @Test
    fun aBandStartsWhereTheFrameLeftOffAndFillsOnlyItsOwnPixels() {
        val rgba = ByteArray(4 * 4) { it.toByte() }
        val colours = IntArray(3) { -1 }
        straightRgbaToArgb(rgba, 1, 2, colours)
        assertEquals(listOf(0x07040506, 0x0B08090A, -1), colours.toList())
    }
}
