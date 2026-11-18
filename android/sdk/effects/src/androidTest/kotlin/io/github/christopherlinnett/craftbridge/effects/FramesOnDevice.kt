// A frame as the pictures Android draws with, against the core's own conversion of the same frame.

package io.github.christopherlinnett.craftbridge.effects

import android.graphics.Bitmap
import android.hardware.HardwareBuffer
import android.os.Build
import io.github.christopherlinnett.craftbridge.core.Engine
import io.github.christopherlinnett.craftbridge.core.UnsupportedException
import io.github.christopherlinnett.craftbridge.ffi.core.GpuChoice
import io.github.christopherlinnett.craftbridge.ffi.core.PixelFormat
import java.nio.ByteBuffer

/** The bytes of one pixel of a raw frame. */
private const val BYTES_PER_PIXEL = 4

/** The side of the square comp the frame checks render. */
private const val PAIRS_SIDE = 256

/**
 * A comp whose frame pairs many 8-bit colours with many 8-bit alphas, in straight colour: a ramp from black
 * to white down the frame, wiped across it with a feather as wide as the frame, so each row's colour meets
 * every alpha the feather makes. Those are the pixels where a straight byte and a premultiplied one differ.
 * The project is 16 bits a channel: an 8-bit project's compositor rounds each premultiplied colour to a byte
 * before the straight frame divides it by alpha, so every colour of its frame times its alpha lands on a
 * byte again and no rounding is ever put to the test.
 */
private const val PAIRS_COMP =
    """{"steps":[
        {"command":"file.projectSettings","params":{"bitDepth":16}},
        {"command":"comp.new","params":{"name":"Pairs","width":$PAIRS_SIDE,"height":$PAIRS_SIDE,"duration":1,"frameRate":30}},
        {"command":"layer.newSolid","params":{"comp":1,"name":"Ramp","color":"#ffffff"}},
        {"command":"effect.apply","params":{"comp":1,"layer":"Ramp","effect":"Gradient Ramp"}},
        {"command":"effect.apply","params":{"comp":1,"layer":"Ramp","effect":"Linear Wipe"}},
        {"command":"prop.set","params":{"layer":"Ramp","path":"effects/#2/completion","value":50}},
        {"command":"prop.set","params":{"layer":"Ramp","path":"effects/#2/feather","value":$PAIRS_SIDE}}
    ]}"""

/** A byte's largest value, which premultiplying divides a colour times its alpha by. */
private const val BYTE_MAX = 255

/** The remainders of a colour times its alpha over [BYTE_MAX] that lie half-way between two bytes. */
private val HALF_WAY = setOf(BYTE_MAX / 2, BYTE_MAX / 2 + 1)

/**
 * How many colour bytes of the frame must make, with their alpha, a product half-way between two bytes:
 * there one rule of rounding and the next part, so a rounding changed by one changes bytes of the frame.
 */
private const val MIN_HALF_WAY = 100

/** The frame request that keeps the straight alpha instead of compositing over the comp's background. */
private const val TRANSPARENT_FRAME = """{"transparent":true}"""

/** The pixels of [this] bitmap as the bytes it holds: premultiplied RGBA, as `copyPixelsToBuffer` copies them. */
private fun Bitmap.storedBytes(): ByteArray = ByteBuffer.allocate(byteCount).also { copyPixelsToBuffer(it) }.array()

/** The premultiplied RGBA bytes of a frame the core answered as premultiplied BGRA: red and blue exchanged. */
private fun ByteArray.bgraAsRgba(): ByteArray =
    ByteArray(size).also { rgba ->
        for (at in indices step BYTES_PER_PIXEL) {
            rgba[at] = this[at + 2]
            rgba[at + 1] = this[at + 1]
            rgba[at + 2] = this[at]
            rgba[at + 3] = this[at + 3]
        }
    }

/**
 * What an app draws a frame with is the core's conversion of it, to the byte. The reference is the core's
 * own premultiplied answer for the same render, so no rule of arithmetic is written here, and the frame
 * is chosen to pair many colours with many alphas, which a mere relabelling or copy of the straight bytes
 * would get wrong, and a rounding off by one too.
 */
internal val frameChecks: List<Case> =
    listOf(
        Case("a frame as a Bitmap is the core's premultiplied conversion, byte for byte") { context ->
            onBoundary(context, GpuChoice.CPU_ONLY) { boundary ->
                val engine = Engine.fromBoundary(boundary)
                engine.request("run", PAIRS_COMP)
                val bitmap = engine.frameBitmap(TRANSPARENT_FRAME)
                val straight = boundary.frame(TRANSPARENT_FRAME, PixelFormat.RGBA8_STRAIGHT)
                val premultiplied = boundary.frame(TRANSPARENT_FRAME, PixelFormat.BGRA8_PREMULTIPLIED).data.bgraAsRgba()
                require(bitmap.config == Bitmap.Config.ARGB_8888 && bitmap.isPremultiplied) {
                    "the bitmap is ${bitmap.config}, premultiplied ${bitmap.isPremultiplied}"
                }
                require(bitmap.width.toUInt() == straight.width && bitmap.height.toUInt() == straight.height) {
                    "the bitmap is ${bitmap.width} by ${bitmap.height}"
                }
                val halfWay =
                    straight.data.indices.count { at ->
                        val alpha = straight.data[at - at % BYTES_PER_PIXEL + 3].toUByte().toInt()
                        at % BYTES_PER_PIXEL != 3 && straight.data[at].toUByte().toInt() * alpha % BYTE_MAX in HALF_WAY
                    }
                require(halfWay >= MIN_HALF_WAY) {
                    "only $halfWay colour bytes of the frame lie half-way under their alpha, too few to pin the rounding"
                }
                val stored = bitmap.storedBytes()
                val differing = stored.indices.count { stored[it] != premultiplied[it] }
                require(
                    differing == 0,
                ) { "$differing of ${stored.size} bytes of the bitmap differ from the core's premultiplied conversion" }
                require(
                    !stored.contentEquals(straight.data),
                ) { "the bitmap holds the straight bytes, which is no premultiplied conversion" }
                bitmap.recycle()
            }
        },
        Case("a frame as a HardwareBuffer holds the bitmap's pixels, or is refused before Android 12") { context ->
            onEngine(context, GpuChoice.CPU_ONLY) { engine ->
                engine.request("run", PAIRS_COMP)
                if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
                    val refused = runCatching { engine.frameHardwareBuffer(TRANSPARENT_FRAME) }.exceptionOrNull()
                    require(refused is UnsupportedException) { "a hardware buffer before Android 12 was answered $refused" }
                    return@onEngine
                }
                val bitmap = engine.frameBitmap(TRANSPARENT_FRAME)
                val buffer = engine.frameHardwareBuffer(TRANSPARENT_FRAME)
                require(
                    buffer.width == bitmap.width && buffer.height == bitmap.height,
                ) { "the buffer is ${buffer.width} by ${buffer.height}" }
                require(buffer.format == HardwareBuffer.RGBA_8888) { "the buffer's format is ${buffer.format}" }
                val read = Bitmap.wrapHardwareBuffer(buffer, null)?.copy(Bitmap.Config.ARGB_8888, false)
                require(read != null) { "the buffer cannot be read back as a bitmap" }
                require(read.storedBytes().contentEquals(bitmap.storedBytes())) { "the buffer's pixels differ from the bitmap's" }
                buffer.close()
                bitmap.recycle()
            }
        },
    )
