// §5's preview and frame steps through `EffectsEngine`'s typed calls, against the core's own calls and
// conversion of the same frame.

package io.github.christopherlinnett.craftbridge.effects

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Color
import android.hardware.HardwareBuffer
import android.media.ImageReader
import android.os.Build
import android.os.Parcel
import android.view.Surface
import io.github.christopherlinnett.craftbridge.core.CraftException
import io.github.christopherlinnett.craftbridge.core.GpuException
import io.github.christopherlinnett.craftbridge.core.InvalidParamsException
import io.github.christopherlinnett.craftbridge.core.UnsupportedException
import io.github.christopherlinnett.craftbridge.core.jsonValue
import io.github.christopherlinnett.craftbridge.ffi.core.GpuChoice
import io.github.christopherlinnett.craftbridge.ffi.core.PixelFormat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
import kotlin.time.Duration.Companion.seconds

/** The frame request the core's own picture calls are given for the first comp, written by hand as the typed calls must not make an app. */
private const val FIRST_COMP_TRANSPARENT = """{"comp":1,"transparent":true}"""

/** The first comp each of these checks makes, as `comp.new` numbers it. */
private val FIRST_COMP = 1.0.jsonValue

/** A comp none of these checks makes, whose frame the engine refuses. */
private val MISSING_COMP = 99.0.jsonValue

/** The side of the second comp, smaller than the first, which the checks make active. */
private const val OTHER_SIDE = 48.0

/**
 * The seconds by which the top layer of the first comp the preview checks make has faded in whole: a part
 * of a second past a whole one, so a preview whose time lost its fraction on the way shows another frame.
 */
private const val LATE = 1.5

/**
 * The two comps the preview checks draw: the first red under a green layer, which [timedComps] fades in from
 * nothing at the start to whole at [LATE], and a blue one, which `comp.new` makes the active comp. So a preview
 * that dropped the comp it names would show the blue, and one that dropped its time the comp at its current time.
 */
private const val TIMED_COMPS =
    """{"steps":[
        {"command":"comp.new","params":{"name":"Timed","width":$SURFACE_SIDE,"height":$SURFACE_SIDE,"duration":2,"frameRate":30}},
        {"command":"layer.newSolid","params":{"name":"Early","color":"#ff0000"}},
        {"command":"layer.newSolid","params":{"name":"Late","color":"#00ff00"}},
        {"command":"comp.new","params":{"name":"Other","width":$SURFACE_SIDE,"height":$SURFACE_SIDE,"duration":2,"frameRate":30}},
        {"command":"layer.newSolid","params":{"name":"Blue","color":"#0000ff"}}
    ]}"""

/** The engine the app starts, with [check] run on it and the engine ended however the check ends. */
private suspend fun withEffects(
    context: Context,
    gpu: GpuChoice,
    check: suspend (EffectsEngine) -> Unit,
) {
    val engine = EffectsEngine.start(context, gpu = gpu)
    try {
        check(engine)
    } finally {
        engine.shutDown()
    }
}

/** The pairs comp as the first comp, then a smaller one, which becomes the active comp, so a call that names the first is told apart. */
private suspend fun EffectsEngine.pairsThenAnotherComp() {
    core.request("run", PAIRS_COMP)
    exec(
        "comp.new",
        mapOf(
            "name" to "Other".jsonValue,
            "width" to OTHER_SIDE.jsonValue,
            "height" to OTHER_SIDE.jsonValue,
            "duration" to 1.0.jsonValue,
            "frameRate" to 30.0.jsonValue,
        ),
    )
}

/** The first comp's frame as the core's own premultiplied answer, in the byte order a bitmap stores. */
private suspend fun EffectsEngine.premultipliedBytes(): ByteArray =
    frame(comp = FIRST_COMP, transparent = true, format = PixelFormat.BGRA8_PREMULTIPLIED).data.bgraAsRgba()

/**
 * The comps of [TIMED_COMPS], with the first one's top layer keyed to fade in by [LATE], and the middle of the
 * core's frame of that comp at [LATE], which the preview checks look for: it is checked to differ from the
 * middle of the same comp at its current time and from the active comp's, so neither could stand in for it.
 */
private suspend fun EffectsEngine.timedComps(): List<Int> {
    core.request("run", TIMED_COMPS)
    for ((time, opacity) in listOf(0.0 to 0.0, LATE to 100.0)) {
        set(layer = "Late".jsonValue, path = "transform/opacity", comp = FIRST_COMP, value = opacity.jsonValue, time = time)
    }
    val late = middleOf(FIRST_COMP, LATE)
    val current = middleOf(FIRST_COMP, null)
    val active = middleOf(null, LATE)
    require(late != current && late != active) {
        "the first comp's middle at $LATE seconds, $late, is not told apart from its middle now, $current, or the active comp's, $active"
    }
    return late
}

/** That the next frame [reader]'s surface shows has the red, green and blue [expected] in its middle, once [what] drew it. */
private fun showed(
    reader: ImageReader,
    expected: List<Int>,
    what: String,
) {
    val shown = middlePixel(reader).take(expected.size)
    require(shown == expected) { "$what drew $shown in the middle, not the core's frame of the comp and time it names, $expected" }
}

/** What [read] makes of [surface] written into a parcel, the way the platform hands a surface from one object to another. */
private fun <T> parcelled(
    surface: Surface,
    read: (Parcel) -> T,
): T {
    val parcel = Parcel.obtain()
    try {
        surface.writeToParcel(parcel, 0)
        parcel.setDataPosition(0)
        return read(parcel)
    } finally {
        parcel.recycle()
    }
}

/** A [Surface] object of its own naming [surface]'s window, which [nameWindowOf] can point at another window later. */
private fun objectNaming(surface: Surface): Surface = parcelled(surface) { Surface.CREATOR.createFromParcel(it) }

/** Point this [Surface] object at [surface]'s window, as a `SurfaceView` points its one object at each surface it makes. */
private fun Surface.nameWindowOf(surface: Surface) {
    parcelled(surface) { readFromParcel(it) }
}

/** That [call] is refused on a comp that does not exist as `frame` refuses that comp: the same row, in the same words. */
private suspend fun EffectsEngine.refusesTheMissingCompAsFrameDoes(
    what: String,
    call: suspend EffectsEngine.() -> Unit,
) {
    val framed = runCatching { frame(comp = MISSING_COMP) }.exceptionOrNull()
    require(framed is CraftException) { "a frame of a comp that does not exist was answered $framed" }
    val answered = runCatching { call() }.exceptionOrNull()
    require(answered is CraftException && answered::class == framed::class && answered.message == framed.message) {
        "$what of a comp that does not exist was answered $answered, not as the frame was: $framed"
    }
}

internal val typedCallChecks: List<Case> =
    listOf(
        Case("a typed preview draws the comp and time it names into the Surface, as the core renders that frame") { context ->
            withEffects(context, GpuChoice.PREFERRED) { engine ->
                val late = engine.timedComps()
                val surface = reader()
                engine.preview(surface.surface, comp = FIRST_COMP, time = LATE.seconds)
                showed(surface, late, "the preview into the Surface")
                engine.preview(comp = FIRST_COMP, time = LATE.seconds)
                showed(surface, late, "the preview into the surface attached")
                engine.preview(surface.surface, comp = FIRST_COMP, time = LATE.seconds)
                showed(surface, late, "the preview into the Surface attached again")
                engine.refusesTheMissingCompAsFrameDoes("a preview") { preview(comp = MISSING_COMP) }
                engine.preview(comp = FIRST_COMP, time = LATE.seconds)
                showed(surface, late, "the preview after a refused one")
                engine.endPreview()
                val refused = runCatching { engine.preview(comp = FIRST_COMP) }.exceptionOrNull()
                require(refused is GpuException) { "a preview with no surface attached was answered $refused, not refused as `Gpu`" }
                surface.close()
            }
        },
        Case("a typed preview answers Gpu with the GPU given up, a picture still renders, and the Surface draws once back") { context ->
            withEffects(context, GpuChoice.PREFERRED) { engine ->
                val late = engine.timedComps()
                val surface = reader()
                engine.preview(surface.surface, comp = FIRST_COMP, time = LATE.seconds)
                showed(surface, late, "the preview before the GPU was given up")
                engine.core.suspendGpu()
                require(!engine.core.hasPreviewSurface()) { "the engine reports a surface the GPU took with it" }
                require(answeredGpu { engine.preview(comp = FIRST_COMP, time = LATE.seconds) } != null) {
                    "a preview after the GPU was given up was not refused as `Gpu`"
                }
                require(answeredGpu { engine.preview(surface.surface, comp = FIRST_COMP, time = LATE.seconds) } != null) {
                    "a preview into the Surface with no device to draw on was not refused as `Gpu`"
                }
                val bitmap = engine.frameBitmap(comp = FIRST_COMP, time = LATE)
                val middle = bitmap.getPixel(bitmap.width / 2, bitmap.height / 2)
                require(listOf(Color.red(middle), Color.green(middle), Color.blue(middle)) == late) {
                    "the bitmap rendered with the GPU given up has $middle in the middle, not the frame's $late"
                }
                bitmap.recycle()
                val note = engine.core.resumeGpu()
                require(note == null) { "the device was not taken again: $note" }
                require(answeredGpu { engine.preview(comp = FIRST_COMP, time = LATE.seconds) } != null) {
                    "the surface came back with the device, which holds only what is attached after it"
                }
                engine.preview(surface.surface, comp = FIRST_COMP, time = LATE.seconds)
                showed(surface, late, "the preview into the Surface once the GPU was back")
                engine.endPreview()
                surface.close()
            }
        },
        Case(
            "a typed preview names its Surface once: it reattaches only when the engine is not previewing into it, and endPreview makes it forget",
        ) { context ->
            withEffects(context, GpuChoice.PREFERRED) { engine ->
                val late = engine.timedComps()
                val held = TextureSurface()
                require(!engine.core.holdsPreview(held.surface)) { "no preview yet, and the engine says it previews into this Surface" }
                engine.preview(held.surface, comp = FIRST_COMP, time = LATE.seconds)
                require(engine.core.holdsPreview(held.surface)) { "the preview attached the Surface, and the engine does not report it" }
                held.texture.setDefaultBufferSize(SURFACE_SIDE, SURFACE_SIDE / 2)
                val outdated = answeredGpu { engine.preview(held.surface, comp = FIRST_COMP, time = LATE.seconds) }
                require(outdated?.detail == RESIZED) {
                    "naming the held Surface again reattached it, answering ${outdated?.detail} instead of the resize's own"
                }
                engine.preview(held.surface, comp = FIRST_COMP, time = LATE.seconds)
                val second = reader()
                engine.preview(second.surface, comp = FIRST_COMP, time = LATE.seconds)
                showed(second, late, "the preview naming a second Surface")
                require(
                    engine.core.holdsPreview(second.surface),
                ) { "the engine does not report previewing into the Surface it last attached" }
                require(
                    !engine.core.holdsPreview(held.surface),
                ) { "the engine still reports previewing into the Surface the second one replaced" }
                engine.endPreview()
                require(!engine.core.hasPreviewSurface()) { "endPreview left the engine holding a surface" }
                require(
                    !engine.core.holdsPreview(second.surface),
                ) { "endPreview left the identity answer still yes for the surface it gave up" }
                held.texture.setDefaultBufferSize(SURFACE_SIDE, SURFACE_SIDE)
                engine.preview(held.surface, comp = FIRST_COMP, time = LATE.seconds)
                require(engine.core.holdsPreview(held.surface)) { "the preview after endPreview did not attach the Surface afresh" }
                held.close()
                second.close()
            }
        },
        Case(
            "a typed preview attaches again a Surface that names another window since, and refuses one the app has released",
        ) { context ->
            withEffects(context, GpuChoice.PREFERRED) { engine ->
                val late = engine.timedComps()
                val first = reader()
                val second = reader()
                val named = objectNaming(first.surface)
                engine.preview(named, comp = FIRST_COMP, time = LATE.seconds)
                showed(first, late, "the preview into the Surface naming the first window")
                require(engine.core.holdsPreview(named)) { "the preview attached the Surface, and the engine does not report it" }
                named.nameWindowOf(second.surface)
                require(
                    !engine.core.holdsPreview(named),
                ) { "the Surface names another window now, and the engine says it previews into it" }
                engine.preview(named, comp = FIRST_COMP, time = LATE.seconds)
                showed(second, late, "the preview into the same Surface object once it named the second window")
                named.release()
                require(!engine.core.holdsPreview(named)) { "the engine says it previews into a Surface the app has released" }
                val refused = runCatching { engine.preview(named, comp = FIRST_COMP, time = LATE.seconds) }.exceptionOrNull()
                require(
                    refused is InvalidParamsException,
                ) { "a preview into a released Surface was answered $refused, not refused as no live window" }
                engine.endPreview()
                first.close()
                second.close()
            }
        },
        Case("two Surfaces attached at once leave the engine previewing into the one it says it holds") { context ->
            withEffects(context, GpuChoice.PREFERRED) { engine ->
                val late = engine.timedComps()
                val readers = listOf(reader(), reader())
                coroutineScope {
                    for (each in readers) {
                        launch(Dispatchers.Default) { engine.core.attachPreview(each.surface) }
                    }
                }
                val held = readers.filter { engine.core.holdsPreview(it.surface) }
                require(held.size == 1) { "${held.size} of the two Surfaces read as the one the engine previews into" }
                engine.preview(comp = FIRST_COMP, time = LATE.seconds)
                showed(held.single(), late, "the preview into the Surface the engine says it holds")
                engine.endPreview()
                readers.forEach(ImageReader::close)
            }
        },
        Case("a frame as a Bitmap through the typed call is the core's premultiplied conversion of the comp it names") { context ->
            withEffects(context, GpuChoice.CPU_ONLY) { engine ->
                engine.pairsThenAnotherComp()
                val bitmap = engine.frameBitmap(comp = FIRST_COMP, transparent = true)
                require(bitmap.config == Bitmap.Config.ARGB_8888 && bitmap.isPremultiplied) {
                    "the bitmap is ${bitmap.config}, premultiplied ${bitmap.isPremultiplied}"
                }
                require(bitmap.width == PAIRS_SIDE && bitmap.height == PAIRS_SIDE) {
                    "the bitmap is ${bitmap.width} by ${bitmap.height}, not the named comp's $PAIRS_SIDE square"
                }
                val stored = bitmap.storedBytes()
                val reference = engine.premultipliedBytes()
                val differing = stored.indices.count { stored[it] != reference[it] }
                require(
                    differing == 0,
                ) { "$differing of ${stored.size} bytes of the bitmap differ from the core's premultiplied conversion" }
                val viaCore = engine.core.frameBitmap(FIRST_COMP_TRANSPARENT)
                require(viaCore.storedBytes().contentEquals(stored)) { "the typed call and the core's own call answer different bitmaps" }
                bitmap.recycle()
                viaCore.recycle()
                engine.refusesTheMissingCompAsFrameDoes("a Bitmap") { frameBitmap(comp = MISSING_COMP).recycle() }
            }
        },
        Case("a frame as a HardwareBuffer through the typed call holds the same pixels, or is refused before Android 12") { context ->
            withEffects(context, GpuChoice.CPU_ONLY) { engine ->
                engine.pairsThenAnotherComp()
                if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
                    val refused = runCatching { engine.frameHardwareBuffer(comp = FIRST_COMP) }.exceptionOrNull()
                    require(refused is UnsupportedException) { "a hardware buffer before Android 12 was answered $refused" }
                    return@withEffects
                }
                val buffer = engine.frameHardwareBuffer(comp = FIRST_COMP, transparent = true)
                require(buffer.width == PAIRS_SIDE && buffer.height == PAIRS_SIDE) { "the buffer is ${buffer.width} by ${buffer.height}" }
                require(buffer.format == HardwareBuffer.RGBA_8888) { "the buffer's format is ${buffer.format}" }
                val read = Bitmap.wrapHardwareBuffer(buffer, null)?.copy(Bitmap.Config.ARGB_8888, false)
                require(read != null) { "the buffer cannot be read back as a bitmap" }
                require(read.storedBytes().contentEquals(engine.premultipliedBytes())) {
                    "the buffer's pixels differ from the core's premultiplied conversion"
                }
                buffer.close()
                read.recycle()
                engine.refusesTheMissingCompAsFrameDoes("a HardwareBuffer") { frameHardwareBuffer(comp = MISSING_COMP).close() }
            }
        },
    )
