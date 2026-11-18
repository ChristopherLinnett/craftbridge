// The engine's calls as coroutines: progress to a lambda, and a cancel the engine obeys.

package io.github.christopherlinnett.craftbridge.core

import android.graphics.Bitmap
import android.hardware.HardwareBuffer
import android.view.Surface
import io.github.christopherlinnett.craftbridge.ffi.core.PixelFormat
import io.github.christopherlinnett.craftbridge.ffi.core.setDeviceCodecs
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import io.github.christopherlinnett.craftbridge.ffi.core.CancelToken as BoundaryCancelToken
import io.github.christopherlinnett.craftbridge.ffi.core.CraftException as BoundaryException
import io.github.christopherlinnett.craftbridge.ffi.core.Engine as BoundaryEngine
import io.github.christopherlinnett.craftbridge.ffi.core.Folders as BoundaryFolders

/**
 * One engine, called as Kotlin coroutines: [request] for the calls that answer at once,
 * [requestWithProgress] for the long ones that report how far they have got, [shutDown] to end it.
 *
 * Every call fails with a [CraftException] and with nothing else, so an app handles a refusal by
 * catching one of the plan's rows and never by guessing what a foreign exception type holds
 * (AGENTS.md §3). Every call can be cancelled as a coroutine, and ends as a cancelled coroutine does,
 * with a `CancellationException`; what the cancel stops differs: a [requestWithProgress] is stopped in
 * the engine itself, as the app would stop it with its own hand, while a [request] the engine has taken
 * runs to its end there and only the wait for its answer stops.
 *
 * The preview and the frames are here too: [attachPreview] and [present] draw into a [Surface] the app
 * holds, and [frameBitmap] and [frameHardwareBuffer] answer a rendered frame as the pictures Android
 * draws with. What is not here is what an app cannot name yet: `mcp` belongs with the plugin that calls
 * it, and the policy that suspends the GPU when the app goes to the background with the issue that gives
 * it somewhere to be used.
 */
public class Engine internal constructor(
    internal val boundary: BoundaryEngine,
) {
    public companion object {
        /**
         * The engine a Craft app plugin started through its own native boundary, as the core's calls
         * take it. A plugin's own start goes through [start], which calls this once it has handed the
         * device's codec list over: an engine started without the list decodes every stream in software.
         * An app is handed an engine and names no boundary.
         */
        public fun fromBoundary(boundary: BoundaryEngine): Engine = Engine(boundary)

        /**
         * Start an engine in [folders] through a Craft app plugin's own native start, [begin], which is handed the
         * folders as the boundary takes them. The device's codec list is read and handed over first, since the
         * start registers the hardware decoders from it: an engine started without it decodes every stream in
         * software, and its `info` says `hardwareDecoding` is false. A plugin's own start calls this; an app is
         * handed an engine and names no boundary.
         *
         * @throws CraftException for whatever [begin] refuses the start with.
         */
        public fun start(
            folders: Folders,
            begin: (BoundaryFolders) -> BoundaryEngine,
        ): Engine {
            setDeviceCodecs(deviceCodecs())
            return fromBoundary(mapped { begin(folders.toFfi()) })
        }
    }

    /**
     * Ask the engine for one method. Its parameters are [params] as JSON text, `{}` when it takes none;
     * the reply is the engine's JSON text.
     *
     * @throws CraftException for whatever the engine answers the call with.
     */
    public suspend fun request(
        method: String,
        params: String = "{}",
    ): String = mapped { boundary.request(method, params) }

    /**
     * Ask the engine for a method that reports progress: `export` is one, and methods that report
     * progress are the only ones the engine reports anything during.
     *
     * Cancelling the coroutine this runs in, before this call or during it, sets the engine's own cancel
     * token, so the work stops at its next beat, or before its first, instead of running on with nobody
     * listening. The call still waits for the engine to answer, because the engine's wind-down is what
     * deletes the files the export had written; the engine stops at its next beat, so the wait is about
     * one beat and not the rest of the export. Then the call ends as a cancelled coroutine does, with a
     * `CancellationException`, whose cause is what the engine answered — [CancelledException] once it
     * stopped — so the scope it runs in reads it as the cancel it is and not as a failure.
     *
     * @param method the method to ask for, one the app answers with progress.
     * @param params its parameters as JSON text, `{}` when it takes none.
     * @param onProgress every report the engine makes, as it makes it.
     * @throws CraftException for anything the engine answers it with, [CancelledException] when
     * something other than this coroutine stopped it, such as [shutDown].
     */
    public suspend fun requestWithProgress(
        method: String,
        params: String = "{}",
        onProgress: ProgressHandler,
    ): String {
        val cancel = BoundaryCancelToken()
        val progress = ProgressOf(onProgress)
        return stopWhenCancelled(stop = { cancel.cancel() }) {
            mapped { boundary.requestWithProgress(method, params, progress, cancel) }
        }
    }

    /**
     * Preview into [surface]: every later [present] draws its frame there, until [detachPreview], another
     * [attachPreview], or the GPU is given up ([suspendGpu]), which drops the surface with the device.
     * Attaching replaces the surface the engine previewed before.
     *
     * The engine holds the window of the surface, not the [Surface] object, and keeps it until it lets
     * the preview go, so the app may release its own surface first; once it has, the next [present]
     * answers [GpuException] and the session is untouched. An app attaches its surface again after
     * [resumeGpu], and whenever a new surface replaces the old one.
     *
     * @throws InvalidParamsException when [surface] is not a live surface, [UnsupportedException] when the
     * native library that holds the windows is not part of the app, [GpuException] when the engine has no
     * device to preview on (it renders on the CPU), or the surface cannot become a preview, as one whose
     * consumer is already gone cannot.
     */
    public suspend fun attachPreview(surface: Surface) {
        attachingWindow(windowOf(surface), NativeSurfaces::release) { window -> mapped { boundary.attachWindow(window) } }
    }

    /**
     * Give up the preview surface, if one is attached: [present] answers [GpuException] again until the
     * app attaches another. Detaching nothing is not an error.
     *
     * @throws CraftException when the engine has been shut down.
     */
    public suspend fun detachPreview() {
        mapped { boundary.detachPreview() }
    }

    /**
     * Draw one frame into the attached surface and show it. [params] are the preview's own, as JSON text:
     * the app plugin says which, and for EffectCraft they are the `frame` request's fields, `comp` and
     * `time` among them, `{}` for the active comp at its current time.
     *
     * A surface the app released, destroyed or resized answers [GpuException] and the session is kept: a
     * resized one is drawn on again by the next call, and a lost one answers every call until the app
     * attaches another [Surface]. With no surface attached it answers [GpuException] too.
     *
     * @throws GpuException for any failure of the surface or the device, [CraftException] for anything
     * else the engine answers the call with.
     */
    public suspend fun present(params: String = "{}") {
        mapped { boundary.present(params) }
    }

    /**
     * Render one frame and answer it as a [Bitmap]: `ARGB_8888`, premultiplied, the pixels the core's
     * own premultiplied conversion gives (plan §17). [params] are the frame's own, as JSON text, `{}` for
     * the active comp at its current time. The conversion runs off the caller's thread.
     *
     * @throws CraftException when the engine refuses the frame, [InternalErrorException] when the phone
     * has no memory for the bitmap.
     */
    public suspend fun frameBitmap(params: String = "{}"): Bitmap = withContext(Dispatchers.Default) { straightFrame(params).toBitmap() }

    /**
     * Render one frame and answer it as a [HardwareBuffer]: `RGBA_8888` and premultiplied, which the GPU
     * and other processes read as it is. The caller closes the buffer. It needs Android 12 (API 31) or
     * newer, the first release in which a bitmap hands its buffer out.
     *
     * @throws UnsupportedException before Android 12, [CraftException] for whatever the engine answers
     * the call with.
     */
    public suspend fun frameHardwareBuffer(params: String = "{}"): HardwareBuffer =
        withContext(Dispatchers.Default) {
            val bitmap = straightFrame(params).toBitmap()
            try {
                bitmap.toHardwareBuffer()
            } finally {
                bitmap.recycle()
            }
        }

    /**
     * Give the GPU up for the background: the engine drops its device and the preview surface, and every
     * render answers on the CPU until [resumeGpu]. Suspending twice is not an error.
     *
     * @throws CraftException when the engine is closed, which cannot give up a device it no longer holds.
     */
    public suspend fun suspendGpu() {
        mapped { boundary.suspendGpu() }
    }

    /**
     * Ask for a device again, on return to the foreground. It answers `null` once the engine holds a
     * device, and the reason there is none when it renders on the CPU, which is no failure. The preview
     * surface is not back: the app calls [attachPreview] again.
     *
     * @throws CraftException for what the engine answers when it cannot take the device.
     */
    public suspend fun resumeGpu(): String? = mapped { boundary.resumeGpu() }

    /** The frame as the core's straight RGBA, which Android turns into its own premultiplied pictures. */
    private suspend fun straightFrame(params: String) = mapped { boundary.frame(params, PixelFormat.RGBA8_STRAIGHT) }

    /**
     * End the engine: its thread finishes, and every later call of this engine answers that it can no
     * longer be answered. An app calls it when it is done with the engine.
     *
     * @throws CraftException if the engine could not be ended.
     */
    public suspend fun shutDown() {
        mapped { boundary.shutDown() }
    }
}

/**
 * Run [call], work the engine does that cannot be cut short from here, and answer what it answers,
 * with a cancellation of the calling coroutine handed to the engine as [stop], the engine's own cancel.
 *
 * The stop has to reach the engine *while the call runs*, so it is set by a child of the call, which
 * begins cancelling as soon as the caller does; waiting for the call to finish would set it only after
 * the work it was meant to stop. The child starts before the call is made, in the caller's own frame
 * rather than through its dispatcher: a child cancelled before its dispatcher starts it never runs at
 * all, so a caller cancelled before the call, or in the moment after, would leave the work to run to its
 * end with nothing to stop it. Once the call has answered, the child is ended too, setting a stop no
 * call reads any more.
 *
 * [call]'s answer is waited for even after the caller was cancelled: the engine winds the work down,
 * and its wind-down is what deletes the files it had written. A caller that was cancelled is then
 * told so as a coroutine is, with a [CancellationException] whose cause is what the engine answered:
 * a coroutine that ends a cancel by throwing anything else fails the scope it runs in, which at the
 * top of an app's scope is an uncaught exception, and on Android that ends the app.
 */
internal suspend fun <R> stopWhenCancelled(
    stop: () -> Unit,
    call: suspend () -> R,
): R =
    try {
        coroutineScope {
            val stopWatching =
                launch(start = CoroutineStart.UNDISPATCHED) {
                    try {
                        awaitCancellation()
                    } catch (cancelled: CancellationException) {
                        stop()
                    }
                }
            try {
                withContext(NonCancellable) { call() }
            } finally {
                // The scope waits for its children, so a child left watching would hold the call open
                // after it had answered.
                stopWatching.cancel()
            }
        }
    } catch (answered: CraftException) {
        // Outside the scope above, which would keep the cancel it was ended with and drop this cause.
        if (!currentCoroutineContext().isActive) {
            throw CancellationException(answered.message, answered)
        }
        throw answered
    }

/**
 * Attach [window] with [attach] and hand its id to [release] whichever way the attach ends: one that
 * took the window left nothing under the id, and one that failed left the window there, which would
 * otherwise be held until the process ends, so the id means the same either way.
 */
internal inline fun attachingWindow(
    window: Long,
    release: (Long) -> Unit,
    attach: (Long) -> Unit,
) {
    try {
        attach(window)
    } finally {
        release(window)
    }
}

/**
 * The id the native library holds the window of [surface] under, which `attachWindow` takes.
 *
 * Refusals are the core's, spelt as the core spells the rows they belong to: a surface the platform has
 * already destroyed names no window, and a build without the native library cannot look one up.
 */
internal fun windowOf(surface: Surface): Long {
    val window =
        try {
            NativeSurfaces.acquire(surface)
        } catch (absent: LinkageError) {
            failWith(BoundaryException.Unsupported("the native library that holds preview windows is not in this app: ${absent.message}"))
        }
    if (window == 0L) {
        failWith(BoundaryException.InvalidParams("the surface is not a live window to preview on"))
    }
    return window
}
