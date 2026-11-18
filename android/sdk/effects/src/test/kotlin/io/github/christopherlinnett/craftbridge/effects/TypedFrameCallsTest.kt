// The preview and the pictures: each typed call hands the core the one frame request its arguments
// make, in the field names the engine's `frame` reads, and as the very text `frame` hands it for the same
// arguments: a request a call spelt for itself differs from that text wherever the two spellings part,
// even where the engine would read them alike.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.ffi.core.PixelFormat
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/** The request every typed call makes of these arguments, written out once for the tests to read against. */
private const val FULL_REQUEST = """{"comp":3.0,"time":1.5,"frame":12.0,"maxSide":512.0,"scale":0.5,"transparent":true}"""

/** Runs a picture call to its end: it converts on another thread, which [await] cannot wait for. */
private fun <R> settle(call: suspend () -> R): Result<R> = runBlocking { runCatching { call() } }

class TypedFrameCallsTest {
    private fun parsed(request: String) = jsonValueOf(request)

    private val comp = JSONValue.Number(3.0)

    @Test
    fun frameBitmapAsksTheCoreForStraightPixelsOfTheRequestFrameMakes() {
        val boundary = RecordingBoundary()
        val engine = boundaryEngine(boundary)
        await { engine.frame(comp = comp, time = 1.5, frame = 12, maxSide = 512, scale = 0.5, transparent = true) }
        val outcome =
            settle {
                engine.frameBitmap(
                    comp = comp,
                    time = 1.5,
                    frame = 12,
                    maxSide = 512,
                    scale = 0.5,
                    transparent = true,
                )
            }
        // The recorded frame is no picture, so the conversion refuses it (as a host with no native library can
        // only say it by failing); what the core was asked is the point.
        assertTrue(outcome.isFailure, "a frame with no pixels is no picture, and the conversion refuses it")
        val framed = boundary.frames.first().first
        assertEquals(parsed(FULL_REQUEST), parsed(framed))
        assertEquals(listOf(framed to PixelFormat.PNG, framed to PixelFormat.RGBA8_STRAIGHT), boundary.frames)
    }

    @Test
    fun frameHardwareBufferAsksTheCoreForStraightPixelsOfTheRequestFrameMakes() {
        val boundary = RecordingBoundary()
        val engine = boundaryEngine(boundary)
        val card = JSONValue.Text("Card")
        await { engine.frame(comp = card, time = 2.0, frame = 3, maxSide = 256, scale = 0.25, transparent = false) }
        settle {
            engine.frameHardwareBuffer(
                comp = card,
                time = 2.0,
                frame = 3,
                maxSide = 256,
                scale = 0.25,
                transparent = false,
            )
        }
        val framed = boundary.frames.first().first
        assertEquals(parsed("""{"comp":"Card","time":2.0,"frame":3.0,"maxSide":256.0,"scale":0.25,"transparent":false}"""), parsed(framed))
        assertEquals(listOf(framed to PixelFormat.PNG, framed to PixelFormat.RGBA8_STRAIGHT), boundary.frames)
    }

    @Test
    fun aPictureCallLeavesOutWhatWasNotGiven() {
        val boundary = RecordingBoundary()
        settle { boundaryEngine(boundary).frameBitmap() }
        assertEquals(listOf("{}"), boundary.frames.map { it.first })
    }

    @Test
    fun frameAsksTheCoreForTheRequestAndTheFormatGiven() {
        val boundary = RecordingBoundary()
        await { boundaryEngine(boundary).frame(comp = comp, time = 1.5, frame = 12, maxSide = 512, scale = 0.5, transparent = true) }
        await { boundaryEngine(boundary).frame(format = PixelFormat.RGBA8_STRAIGHT) }
        assertEquals(
            listOf(parsed(FULL_REQUEST) to PixelFormat.PNG, parsed("{}") to PixelFormat.RGBA8_STRAIGHT),
            boundary.frames.map { parsed(it.first) to it.second },
        )
    }

    @Test
    fun previewDrawsTheCompAtTheTimeIntoTheAttachedSurface() {
        val boundary = RecordingBoundary()
        val engine = boundaryEngine(boundary)
        await { engine.frame(comp = comp, time = 1.5) }
        await { engine.preview(comp = comp, time = 1.5) }
        val framed = boundary.frames.first().first
        assertEquals(JSONValue.Object(mapOf("comp" to comp, "time" to JSONValue.Number(1.5))), parsed(framed))
        assertEquals(listOf(framed), boundary.presents)
    }

    @Test
    fun previewWithNoArgumentsDrawsTheActiveCompAtItsCurrentTime() {
        val boundary = RecordingBoundary()
        await { boundaryEngine(boundary).preview() }
        assertEquals(listOf("{}"), boundary.presents)
    }
}
