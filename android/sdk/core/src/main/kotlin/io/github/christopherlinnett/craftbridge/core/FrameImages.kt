// A frame the engine rendered, as the pictures an Android app draws with.

package io.github.christopherlinnett.craftbridge.core

import android.graphics.Bitmap
import android.hardware.HardwareBuffer
import android.os.Build
import io.github.christopherlinnett.craftbridge.ffi.core.FrameBytes
import io.github.christopherlinnett.craftbridge.ffi.core.PixelFormat
import io.github.christopherlinnett.craftbridge.ffi.core.CraftException as BoundaryException

/** The bytes of one pixel of a raw frame: red, green, blue and alpha. */
private const val BYTES_PER_PIXEL = 4

/**
 * How many pixels one `setPixels` call converts at most. The call takes colours as an `IntArray`, so a
 * whole frame at once would hold a second copy of it as large as the frame (33 MB at 4K); a band of
 * rows is converted and handed over at a time, which keeps that copy the size of the band.
 */
private const val BAND_PIXELS = 1 shl 20

/** The bitmap's colour mask for the alpha, red, green and blue bytes of an `ARGB` colour, from the top. */
private const val ALPHA_SHIFT = 24
private const val RED_SHIFT = 16
private const val GREEN_SHIFT = 8

/** The 8 bits a byte holds, as `Byte.toInt` sign-extends them. */
private const val BYTE_MASK = 0xFF

/**
 * The premultiplied `ARGB_8888` [Bitmap] this straight RGBA frame is.
 *
 * The core answers an Android app the straight frame (`RGBA8_STRAIGHT`, plan §17), and `setPixels` is the
 * call that makes a premultiplied bitmap of straight colours: it premultiplies each as it stores it, to the
 * bytes the core's own `BGRA8_PREMULTIPLIED` conversion writes (the device checks pin that, byte for
 * byte). `setPremultiplied` and `copyPixelsFromBuffer` convert nothing — the first only labels the
 * pixels the bitmap holds, the second copies bytes as they are — so straight bytes handed to them are
 * shown as premultiplied ones, too bright wherever the frame is translucent.
 */
internal fun FrameBytes.toBitmap(): Bitmap {
    val width = width.toInt()
    val height = height.toInt()
    if (format != PixelFormat.RGBA8_STRAIGHT || data.size.toLong() != width.toLong() * height * BYTES_PER_PIXEL) {
        failWith(
            BoundaryException.Internal(
                "a ${this.width} by ${this.height} frame of $format with ${data.size} bytes is not straight RGBA pixels",
            ),
        )
    }
    val bitmap =
        try {
            Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
        } catch (tooBig: OutOfMemoryError) {
            failWith(BoundaryException.Internal("the phone has no memory for a $width by $height bitmap: ${tooBig.message}"))
        }
    val rowsPerBand = maxOf(1, BAND_PIXELS / width)
    val band = IntArray(rowsPerBand * width)
    var top = 0
    while (top < height) {
        val rows = minOf(rowsPerBand, height - top)
        straightRgbaToArgb(data, top * width, rows * width, band)
        bitmap.setPixels(band, 0, width, 0, top, width, rows)
        top += rows
    }
    return bitmap
}

/**
 * Fill [into] with the `ARGB` colours of the [count] pixels of [rgba] that start at pixel [first]: the
 * straight bytes red, green, blue, alpha become the colour int `setPixels` reads, alpha in the top byte.
 */
internal fun straightRgbaToArgb(
    rgba: ByteArray,
    first: Int,
    count: Int,
    into: IntArray,
) {
    for (pixel in 0 until count) {
        val at = (first + pixel) * BYTES_PER_PIXEL
        into[pixel] =
            ((rgba[at + 3].toInt() and BYTE_MASK) shl ALPHA_SHIFT) or
            ((rgba[at].toInt() and BYTE_MASK) shl RED_SHIFT) or
            ((rgba[at + 1].toInt() and BYTE_MASK) shl GREEN_SHIFT) or
            (rgba[at + 2].toInt() and BYTE_MASK)
    }
}

/**
 * The [HardwareBuffer] this bitmap's pixels are copied into, which the GPU and other processes can read
 * as it is: an `RGBA_8888` buffer of the bitmap's premultiplied bytes. The bitmap is left as it was.
 *
 * A bitmap hands out its buffer from Android 12 (API 31) on; before that the platform has no call that
 * does, and an app on such a phone is told so.
 */
internal fun Bitmap.toHardwareBuffer(): HardwareBuffer {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
        failWith(BoundaryException.Unsupported("a frame as a HardwareBuffer needs Android 12 (API 31) or newer"))
    }
    val hardware =
        copy(Bitmap.Config.HARDWARE, false)
            ?: failWith(BoundaryException.Internal("the phone could not copy the frame into a hardware bitmap"))
    try {
        return hardware.hardwareBuffer
    } finally {
        // The buffer is a reference of its own to the pixels, which outlives the bitmap that handed it out.
        hardware.recycle()
    }
}
