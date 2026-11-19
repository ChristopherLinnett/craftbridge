// The preview into a real Surface: the engine draws on it, loses it when the app abandons it, and is
// given another after the GPU was given up and asked for again.

package io.github.christopherlinnett.craftbridge.effects

import android.content.Context
import android.graphics.PixelFormat
import android.graphics.SurfaceTexture
import android.media.ImageReader
import android.os.SystemClock
import android.view.Surface
import io.github.christopherlinnett.craftbridge.core.Engine
import io.github.christopherlinnett.craftbridge.core.GpuException
import io.github.christopherlinnett.craftbridge.core.InvalidParamsException
import io.github.christopherlinnett.craftbridge.ffi.core.CraftException
import io.github.christopherlinnett.craftbridge.ffi.core.GpuChoice
import io.github.christopherlinnett.craftbridge.ffi.core.windowAddress

/** The side of the square surfaces the checks preview into, in pixels. */
internal const val SURFACE_SIDE = 64

/** How many presents on an abandoned surface a check makes before it calls the loss unreported. */
private const val PRESENTS_TO_FIND_A_LOSS = 10

/** How long a check waits for a frame it presented to arrive at the surface's reader. */
internal const val WAIT_FOR_FRAME_MILLIS = 30_000L

/** How many of the first window ids a check asks the address of: far more than the checks of one run hand out. */
private const val WINDOW_IDS_ASKED = 4096L

/** What a present answers the first time it finds its surface a new size, in the core's words. */
internal const val RESIZED = "the preview surface changed size or format; it is configured again, so the next frame draws"

/** A comp that is one flat red, so the pixel in the middle of any drawing of it is known. */
private const val RED_COMP =
    """{"steps":[
        {"command":"comp.new","params":{"name":"Red","width":$SURFACE_SIDE,"height":$SURFACE_SIDE,"duration":1,"frameRate":30}},
        {"command":"layer.newSolid","params":{"comp":1,"name":"Bg","color":"#ff0000"}}
    ]}"""

/** The pixel red and opaque makes, as an `RGBA_8888` buffer holds it. */
private val OPAQUE_RED = listOf(255, 0, 0, 255)

/**
 * A surface an app could hand over, with its consumer in hand: an image reader is the platform's own
 * stand-in for a view's surface, whose buffers the check can read, and which the check abandons by
 * closing it, as an app does when its view goes away. It is [side] pixels square.
 */
internal fun reader(side: Int = SURFACE_SIDE): ImageReader = ImageReader.newInstance(side, side, PixelFormat.RGBA_8888, 3)

/**
 * A surface whose consumer is a [SurfaceTexture] the check holds: the platform's stand-in for a
 * `TextureView`'s, whose size the consumer sets and changes, and which the check abandons by releasing it.
 */
internal class TextureSurface {
    val texture = SurfaceTexture(false).apply { setDefaultBufferSize(SURFACE_SIDE, SURFACE_SIDE) }
    val surface = Surface(texture)

    fun close() {
        surface.release()
        texture.release()
    }
}

/** The middle pixel of the next buffer a presented frame left at [reader], waiting for it to arrive. */
internal fun middlePixel(reader: ImageReader): List<Int> {
    val until = SystemClock.elapsedRealtime() + WAIT_FOR_FRAME_MILLIS
    while (SystemClock.elapsedRealtime() < until) {
        reader.acquireNextImage()?.use { image ->
            val plane = image.planes.first()
            val middle = SURFACE_SIDE / 2 * plane.rowStride + SURFACE_SIDE / 2 * 4
            return List(4) { plane.buffer.get(middle + it).toInt() and 0xFF }
        }
        Thread.sleep(50)
    }
    error("no frame reached the surface in ${WAIT_FOR_FRAME_MILLIS / 1000} seconds")
}

/** Present until a frame is drawn, then confirm it is the comp's red at the middle of [reader]'s surface. */
private suspend fun Engine.drawsRedOn(reader: ImageReader) {
    present()
    val pixel = middlePixel(reader)
    require(pixel == OPAQUE_RED) { "the frame drawn on the surface has $pixel in the middle, not opaque red" }
}

/** What a call that must answer [GpuException] answered, which is its failure or the word that it succeeded. */
internal suspend fun answeredGpu(call: suspend () -> Unit): GpuException? = runCatching { call() }.exceptionOrNull() as? GpuException

/** The engine on the device's GPU with the red comp made, which every preview check starts from. */
private suspend fun previewing(
    context: Context,
    check: suspend (Engine) -> Unit,
) = onEngine(context, GpuChoice.PREFERRED) { engine ->
    engine.request("run", RED_COMP)
    check(engine)
}

internal val previewChecks: List<Case> =
    listOf(
        Case("a preview is drawn into the app's Surface, and detaching it ends the preview with the session kept") { context ->
            previewing(context) { engine ->
                val surface = reader()
                engine.attachPreview(surface.surface)
                engine.drawsRedOn(surface)
                engine.detachPreview()
                require(answeredGpu { engine.present() } != null) { "a present after the detach was not refused as `Gpu`" }
                surface.close()
            }
        },
        Case("a Surface the app has already released is refused as no window to preview on") { context ->
            previewing(context) { engine ->
                val surface = reader()
                val released = surface.surface
                surface.close()
                released.release()
                val refused = runCatching { engine.attachPreview(released) }.exceptionOrNull()
                require(refused is InvalidParamsException) { "a released surface was answered $refused, not refused as no window" }
                require(refused.detail == "the surface is not a live window to preview on") { "the refusal reads ${refused.detail}" }
            }
        },
        Case("a Surface the app abandons answers Gpu with the session kept, and the app attaches another") { context ->
            previewing(context) { engine ->
                val abandoned = reader()
                engine.attachPreview(abandoned.surface)
                engine.drawsRedOn(abandoned)
                abandoned.close()
                // The platform reports the loss on a present that finds it; every present after that
                // answers the same until the app attaches a surface that lives.
                var lost: GpuException? = null
                repeat(PRESENTS_TO_FIND_A_LOSS) { if (lost == null) lost = answeredGpu { engine.present() } }
                require(lost != null) { "$PRESENTS_TO_FIND_A_LOSS presents on an abandoned surface were never refused as `Gpu`" }
                require(answeredGpu { engine.present() } != null) { "a present after the loss was answered, with no surface to draw on" }
                require(engine.request("info").contains("activeComp")) { "the session did not answer after the surface was lost" }
                val replacement = reader()
                engine.attachPreview(replacement.surface)
                engine.drawsRedOn(replacement)
                engine.detachPreview()
                replacement.close()
            }
        },
        Case("the GPU given up drops the surface, and the app attaches again after it is asked for again") { context ->
            previewing(context) { engine ->
                val first = reader()
                engine.attachPreview(first.surface)
                engine.drawsRedOn(first)
                engine.suspendGpu()
                require(answeredGpu { engine.present() } != null) { "a present after the GPU was given up was not refused as `Gpu`" }
                val note = engine.resumeGpu()
                require(note == null) { "the device was not taken again: $note" }
                require(
                    answeredGpu { engine.present() } != null,
                ) { "the surface came back with the device, which holds only what is attached after it" }
                first.close()
                val second = reader()
                engine.attachPreview(second.surface)
                engine.drawsRedOn(second)
                engine.detachPreview()
                second.close()
            }
        },
        Case("a Surface whose consumer is gone before it is attached is refused as Gpu, and the app attaches another") { context ->
            previewing(context) { engine ->
                val gone = TextureSurface()
                gone.texture.release()
                val refused = runCatching { engine.attachPreview(gone.surface) }.exceptionOrNull()
                require(refused is GpuException) { "a Surface with no consumer was answered $refused, not refused as `Gpu`" }
                require(refused.detail.startsWith("the window cannot become a preview surface: ")) { "the refusal reads ${refused.detail}" }
                gone.close()
                require(engine.request("info").contains("activeComp")) { "the session did not answer after the refusal" }
                val live = reader()
                engine.attachPreview(live.surface)
                engine.drawsRedOn(live)
                engine.detachPreview()
                live.close()
            }
        },
        Case("a Surface its consumer resizes is configured again: one present answers Gpu, and the next is drawn") { context ->
            previewing(context) { engine ->
                val resized = TextureSurface()
                engine.attachPreview(resized.surface)
                engine.present()
                resized.texture.setDefaultBufferSize(SURFACE_SIDE / 2, SURFACE_SIDE)
                val outdated = answeredGpu { engine.present() }
                require(outdated?.detail == RESIZED) {
                    "the first present after the resize answered ${outdated?.detail}, not that it was resized"
                }
                engine.present()
                engine.detachPreview()
                resized.close()
            }
        },
        Case("an id that names no window answers NotFound and never a crash, and no preview leaves a window under an id") { context ->
            previewing(context) { engine ->
                val surface = reader()
                engine.attachPreview(surface.surface)
                require(engine.holdsPreview(surface.surface)) { "the engine does not report the surface it attached" }
                engine.detachPreview()
                surface.close()
                // The attach took its window out of the table and the hold check gave its own id back, so
                // every id the run handed out, and any that never was, names no window now.
                val named =
                    (listOf(0L, -1L, Long.MIN_VALUE, Long.MAX_VALUE) + (1L..WINDOW_IDS_ASKED)).filter { id ->
                        val answer = runCatching { windowAddress(id) }
                        require(answer.isSuccess || answer.exceptionOrNull() is CraftException.NotFound) {
                            "the address of window id $id was answered ${answer.exceptionOrNull()}, not NotFound"
                        }
                        answer.isSuccess
                    }
                require(
                    named.isEmpty(),
                ) { "${named.size} ids, ${named.take(3)} first, still name a window the engine and the app have both let go" }
            }
        },
        Case("the engine reports the preview surface it holds, from its attach until its detach, or the GPU's going") { context ->
            previewing(context) { engine ->
                require(!engine.hasPreviewSurface()) { "the engine reports holding a surface before one is attached" }
                val surface = reader()
                engine.attachPreview(surface.surface)
                require(engine.hasPreviewSurface()) { "the engine does not report the surface it attached" }
                engine.suspendGpu()
                require(!engine.hasPreviewSurface()) { "the engine still reports the surface the GPU took with it" }
                engine.resumeGpu()
                require(!engine.hasPreviewSurface()) { "the device came back, and no surface came back with it" }
                engine.attachPreview(surface.surface)
                require(engine.hasPreviewSurface()) { "the second attach is held as the first was" }
                engine.detachPreview()
                require(!engine.hasPreviewSurface()) { "the detach gave the surface up, and the report says so" }
                surface.close()
            }
        },
    )
