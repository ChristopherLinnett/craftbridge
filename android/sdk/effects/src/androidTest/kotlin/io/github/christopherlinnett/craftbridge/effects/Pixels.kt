// PNGs of straight 8-bit RGBA, written and read as the files hold them, and two of them held to a
// tolerance as the desktop runner holds them, in its words: the compositor's rule for a render on the GPU
// against the CPU's, which a phone can make against its own CPU's render.

package io.github.christopherlinnett.craftbridge.effects

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import java.io.ByteArrayOutputStream
import java.io.File
import java.nio.ByteBuffer
import java.util.Locale

/** A raster of straight 8-bit RGBA: four bytes a pixel, red first, the levels a PNG file holds. */
internal class Raster(
    val width: Int,
    val height: Int,
    val pixels: ByteArray,
)

/**
 * A PNG holding [pixels], straight RGBA of [width] by [height], exactly as they are. The bitmap is told
 * its pixels are not premultiplied, so the encoder writes the levels it is given rather than scaling them
 * by their alpha.
 */
internal fun pngOfStraight(
    pixels: ByteArray,
    width: Int,
    height: Int,
): ByteArray {
    val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
    try {
        bitmap.isPremultiplied = false
        bitmap.copyPixelsFromBuffer(ByteBuffer.wrap(pixels))
        val encoded = ByteArrayOutputStream()
        if (!bitmap.compress(Bitmap.CompressFormat.PNG, 100, encoded)) {
            throw RunnerFailure("cannot encode the frame")
        }
        return encoded.toByteArray()
    } finally {
        bitmap.recycle()
    }
}

/**
 * A PNG's pixels as the straight 8-bit RGBA the file holds, with its size. The decoder is told not to
 * premultiply, so a translucent pixel keeps its levels; an RGB picture is given full alpha. A PNG of more than
 * 8 bits a channel is handed over as half floats, a layout the tolerance's levels are not counted in, so it is
 * refused, as the Swift runner refuses it.
 */
internal fun straightRaster(file: File): Raster {
    val options = BitmapFactory.Options().apply { inPremultiplied = false }
    val bitmap = BitmapFactory.decodeFile(file.path, options) ?: throw RunnerFailure("cannot read ${file.path}")
    try {
        // Any other layout is refused rather than converted: a conversion would move the levels a tolerance counts.
        if (bitmap.config != Bitmap.Config.ARGB_8888 || bitmap.isPremultiplied) {
            throw RunnerFailure(
                "cannot read ${file.path} as straight 8-bit RGBA: ${bitmap.config}, premultiplied ${bitmap.isPremultiplied}",
            )
        }
        val pixels = ByteArray(bitmap.width * bitmap.height * 4)
        bitmap.copyPixelsToBuffer(ByteBuffer.wrap(pixels))
        return Raster(bitmap.width, bitmap.height, pixels)
    } finally {
        bitmap.recycle()
    }
}

/**
 * The failures of two PNGs' pixels, [actual] against [reference], as the desktop runner words them: each
 * pixel is over when any channel is further than the tolerance's levels from the reference's, no more than
 * its fraction of the pixels may be over, and none may be further than its cap.
 */
internal fun pixelFailures(
    actual: File,
    reference: File,
    tolerance: Tolerance,
): List<String> {
    val (ours, theirs) =
        try {
            straightRaster(actual) to straightRaster(reference)
        } catch (unreadable: RunnerFailure) {
            return listOf(unreadable.description)
        }
    if (ours.width != theirs.width || ours.height != theirs.height) {
        return listOf("size ${ours.width}x${ours.height} is not the reference's ${theirs.width}x${theirs.height}")
    }
    var over = 0
    var furthest = 0
    var total = 0
    for (start in ours.pixels.indices step 4) {
        var worst = 0
        for (channel in start until start + 4) {
            worst = maxOf(worst, kotlin.math.abs((ours.pixels[channel].toInt() and 0xff) - (theirs.pixels[channel].toInt() and 0xff)))
        }
        furthest = maxOf(furthest, worst)
        if (worst > tolerance.levels) {
            over += 1
        }
        total += 1
    }
    val fraction = if (total == 0) 0.0 else over.toDouble() / total
    val failures = mutableListOf<String>()
    if (fraction > tolerance.allowedFraction) {
        val found = String.format(Locale.ROOT, "%.3f", fraction * 100)
        val allowed = String.format(Locale.ROOT, "%.3f", tolerance.allowedFraction * 100)
        failures +=
            "$over of $total pixels are more than ${tolerance.levels} levels from the reference's: $found% where $allowed% is allowed"
    }
    if (furthest > tolerance.cap) {
        failures += "a pixel is $furthest levels from the reference's, past the ${tolerance.cap} allowed"
    }
    return failures
}
