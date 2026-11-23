// The boundary's whole surface, called once each: what this check must call is the manifest the
// `boundary-surface` generator writes from the distribution library's own UniFFI metadata, packed
// into the test APK's assets beside the corpus, so an export the library gains joins the list with
// no edit here — and a check that never calls it fails the run naming it, before the release does.
// Each answer is held to what the core says it answers, which is what a wrong answer fails on.

package io.github.christopherlinnett.craftbridge.effects

import android.content.Context
import android.util.Log
import io.github.christopherlinnett.craftbridge.core.Engine
import io.github.christopherlinnett.craftbridge.core.Folders
import io.github.christopherlinnett.craftbridge.ffi.core.CancelToken
import io.github.christopherlinnett.craftbridge.ffi.core.CraftException
import io.github.christopherlinnett.craftbridge.ffi.core.GpuChoice
import io.github.christopherlinnett.craftbridge.ffi.core.PixelFormat
import io.github.christopherlinnett.craftbridge.ffi.core.PreviewAlpha
import io.github.christopherlinnett.craftbridge.ffi.core.ProgressListener
import io.github.christopherlinnett.craftbridge.ffi.core.attachMetalLayer
import io.github.christopherlinnett.craftbridge.ffi.core.craftErrorMessage
import io.github.christopherlinnett.craftbridge.ffi.core.setDeviceCodecs
import io.github.christopherlinnett.craftbridge.ffi.core.windowAddress
import io.github.christopherlinnett.craftbridge.ffi.effects.startEffectsEngine
import org.json.JSONObject
import java.io.File
import java.util.Collections
import io.github.christopherlinnett.craftbridge.ffi.core.Engine as BoundaryEngine

/** The manifest's path among the test APK's assets: the corpus folder's own `generated` subfolder. */
private const val SURFACE_ASSET = "generated/boundary-surface.json"

/** An id no window of this process ever holds: the table hands out small counting ids, never this. */
private const val NO_SUCH_ID = 999_999L

/** The core's words for an id its window table does not hold, as the host's own platform test pins them. */
private const val NO_WINDOW_WORDS = "no window of this process has that id"

/** The note the core answers when the app asked for no GPU, which a CPU-only engine keeps getting. */
private const val CPU_ONLY_NOTE = "the app asked for CPU-only rendering"

/** The core's words for a call made after its engine was shut down. */
private const val CLOSED_WORDS = "engine is closed"

/** One entry of the manifest: what one call of this check must account for. */
private class SurfaceEntry(
    val namespace: String,
    val kind: String,
    val owner: String?,
    val name: String,
) {
    /** The callable's id: the owner before the name, as the generator's own ids are built. */
    val id: String
        get() = owner?.let { "$it.$name" } ?: name
}

/** The manifest the generator wrote from the library's metadata, read whole from the APK's assets. */
private fun surfaceManifest(context: Context): List<SurfaceEntry> {
    val text = context.assets.open(SURFACE_ASSET).use { it.readBytes().decodeToString() }
    val entries = JSONObject(text).getJSONArray("surface")
    return (0 until entries.length()).map { at ->
        val entry = entries.getJSONObject(at)
        SurfaceEntry(
            namespace = entry.getString("namespace"),
            kind = entry.getString("kind"),
            owner = if (entry.isNull("owner")) null else entry.getString("owner"),
            name = entry.getString("name"),
        )
    }
}

/** The reports an export makes to its listener, taken as they come in, on the engine's own thread. */
private class Hearing : ProgressListener {
    val reports: MutableList<Triple<ULong, ULong, Double?>> = Collections.synchronizedList(mutableListOf())

    override fun progress(
        done: ULong,
        total: ULong,
        secondsLeft: Double?,
    ) {
        reports += Triple(done, total, secondsLeft)
    }
}

/**
 * Makes [call] and records [id] as made, whether the call answers or throws: a call and its record are one
 * statement, so a call taken out takes its record with it, and the check fails for the callable that no
 * statement makes any more.
 */
private inline fun <T> MutableSet<String>.made(
    id: String,
    call: () -> T,
): T =
    try {
        call()
    } finally {
        add(id)
    }

internal val boundarySmokeChecks: List<Case> =
    listOf(
        Case("the smoke checks call every callable of the boundary, as the library's own metadata lists them") { context ->
            val manifest = surfaceManifest(context)
            val called = mutableSetOf<String>()
            lateinit var boundary: BoundaryEngine
            val engine =
                Engine.start(Folders.appDefault(context)) { folders ->
                    // Handed over before the start, as the start reads it: an empty list says this
                    // engine decodes nothing, which is true of it, and making the call is the point.
                    called.made("set_device_codecs") { setDeviceCodecs(emptyList()) }
                    called.made("start_effects_engine") { startEffectsEngine(folders, GpuChoice.CPU_ONLY) }.also { boundary = it }
                }
            try {
                // The cancel token: its constructor, and both of its methods. A fresh one is not set,
                // and once it is the engine would stop any call carrying it — nothing here is stopped.
                val cancel = called.made("CancelToken.new") { CancelToken() }
                val setAtFirst = called.made("CancelToken.is_cancelled") { cancel.isCancelled() }
                require(!setAtFirst) { "a fresh cancel token answered that it was set" }
                called.made("CancelToken.cancel") { cancel.cancel() }
                require(cancel.isCancelled()) { "a cancelled token answered that it was not set" }

                // Recovery: a start that found nothing to clean says so with empty lists, which is
                // what the host's own surface test pins, and the SDK answers it straight through from
                // the boundary. It answers the same after the engine is ended, in the end below.
                val fresh = called.made("Engine.recovery") { boundary.recovery() }
                require(
                    fresh.removed.isEmpty() && fresh.untouched.isEmpty() && fresh.refused.isEmpty() &&
                        fresh.failed.isEmpty() && fresh.journal.isEmpty(),
                ) { "a clean start's recovery was not empty: $fresh" }
                require(engine.recovery() == fresh) { "the SDK's recovery did not answer what the boundary answered" }

                // One request opens the comp every later call of this check draws its frame from.
                called.made("Engine.request") { boundary.request("run", compRequest(EXPORT_FRAMES)) }

                // One frame, asked small: what the core says it answers is the frame scaled so its
                // longer side is the side asked for, in exactly the format the call asked for.
                val frame = called.made("Engine.frame") { boundary.frame("""{"time":0,"maxSide":64}""", PixelFormat.RGBA8_STRAIGHT) }
                require(frame.format == PixelFormat.RGBA8_STRAIGHT) { "the frame came back as ${frame.format}" }
                require(frame.width == 64u && frame.height == 36u) {
                    "the comp is 640x360, so its frame at maxSide 64 is 64x36, not ${frame.width}x${frame.height}"
                }
                require(frame.data.size == 64 * 36 * 4) { "the frame's bytes are not one RGBA pixel each: ${frame.data.size}" }

                // The preview, from an engine holding nothing: the engine's own answer is no surface,
                // detaching nothing is fine, and an id of no window is the core's NotFound on Android.
                val previewing = called.made("Engine.has_preview_surface") { boundary.hasPreviewSurface() }
                require(!previewing) { "a fresh engine answered that it previews" }
                called.made("Engine.detach_preview") { boundary.detachPreview() }
                val noSurface = runCatching { called.made("Engine.present") { boundary.present("{}") } }.exceptionOrNull()
                require(
                    noSurface is CraftException.Gpu && noSurface.detail == "no surface",
                ) { "a present with nothing attached answered $noSurface, not the core's Gpu \"no surface\"" }
                val noWindow = runCatching { called.made("Engine.attach_window") { boundary.attachWindow(NO_SUCH_ID) } }.exceptionOrNull()
                require(
                    noWindow is CraftException.NotFound && noWindow.detail == NO_WINDOW_WORDS,
                ) { "attaching an id of no window answered $noWindow, not the core's NotFound" }
                // The attach that asks to carry the composition's alpha looks its window up as the plain
                // one does, before the ask is read, so an id of no window answers the same whatever it asks.
                val noAlphaWindow =
                    runCatching {
                        called.made("Engine.attach_window_with_alpha") {
                            boundary.attachWindowWithAlpha(NO_SUCH_ID, PreviewAlpha.PREMULTIPLIED)
                        }
                    }.exceptionOrNull()
                require(
                    noAlphaWindow is CraftException.NotFound && noAlphaWindow.detail == NO_WINDOW_WORDS,
                ) { "attaching an id of no window to carry alpha answered $noAlphaWindow, not the core's NotFound" }
                val noAddress = runCatching { called.made("window_address") { windowAddress(NO_SUCH_ID) } }.exceptionOrNull()
                require(
                    noAddress is CraftException.NotFound && noAddress.detail == NO_WINDOW_WORDS,
                ) { "the address of an id of no window answered $noAddress, not the core's NotFound" }
                val noLayer = runCatching { called.made("attach_metal_layer") { attachMetalLayer(boundary, 0uL) } }.exceptionOrNull()
                require(
                    noLayer is CraftException.Unsupported && noLayer.detail == "this platform has no CAMetalLayer to preview on",
                ) { "the Metal attach on Android answered $noLayer, not the core's Unsupported" }

                // The GPU: an engine started for the CPU suspends and resumes, and the resume answers
                // the CPU's own reason, which is what the core says it answers when no device came.
                called.made("Engine.suspend_gpu") { boundary.suspendGpu() }
                val note = called.made("Engine.resume_gpu") { boundary.resumeGpu() }
                require(note == CPU_ONLY_NOTE) { "resuming a CPU-only engine answered $note, not the core's note" }

                // The error's words come from the core itself: the cancelled call's whole sentence is
                // the core's, and a message variant answers its own message back as the core writes it.
                val cancelled = called.made("craft_error_message") { craftErrorMessage(CraftException.Cancelled()) }
                require(cancelled == "cancelled") { "the core did not call a cancelled call \"cancelled\": $cancelled" }
                val found = craftErrorMessage(CraftException.NotFound("nothing like it is here"))
                require(found == "nothing like it is here") { "a message variant did not answer its message back: $found" }

                // One MCP line answers its reply, the engine's tools, each named.
                val reply = called.made("Engine.mcp") { boundary.mcp("""{"jsonrpc":"2.0","id":1,"method":"tools/list"}""") }
                val tools = reply?.let { JSONObject(it).optJSONObject("result")?.optJSONArray("tools") }
                val named = tools != null && tools.length() > 0 && (0 until tools.length()).all { tools.getJSONObject(it).has("name") }
                require(named) { "the MCP tools/list answered no named tools: $reply" }

                // The export, with its listener and its token. The core reports on its beat, every
                // progress interval, while there is work: a report before the render has counted its
                // frames says none of none, and an export a fast device ends inside one beat may never
                // say more, so each report counts nothing yet or this comp's frames, and the work is
                // held by what was written and by the render the call answers, which names a sequence by
                // the file of its first frame.
                val output = exportInto(context, "kotlin-boundary-smoke.png")
                removeExports(output, "kotlin-boundary-smoke")
                val heard = Hearing()
                val answered =
                    called.made("Engine.request_with_progress") {
                        boundary.requestWithProgress("export", exportRequest(output), heard, CancelToken())
                    }
                val reports = heard.reports.toList()
                // The listener's method is the app's own, which the library calls: what it heard is the record.
                if (reports.isNotEmpty()) {
                    called += "ProgressListener.progress"
                }
                Log.i(LOG_TAG, "the smoke export reported ${reports.size} beats: $reports; it answered $answered")
                require(reports.isNotEmpty()) { "the export reported no progress at all" }
                require(reports.all { (done, total, _) -> (total == 0uL || total == EXPORT_FRAMES.toULong()) && done <= total }) {
                    "a report counted another comp than the $EXPORT_FRAMES frames exported, or done past total: $reports"
                }
                val rendered = JSONObject(answered).optJSONArray("rendered")
                val first = File(output.parentFile, frameName(output, 0)).path
                require(rendered != null && rendered.length() == 1 && rendered.getJSONObject(0).optString("output") == first) {
                    "the export did not answer the one render it made, of the sequence starting at $first: $answered"
                }
                val written = exportsNamed(output, "kotlin-boundary-smoke").count { it.isFile }
                require(written == EXPORT_FRAMES) { "the export wrote $written of $EXPORT_FRAMES frames" }
                removeExports(output, "kotlin-boundary-smoke")

                // Ending is not an error twice, every call after it is refused in the core's words, and
                // recovery is what the start already did: it answers the same once the engine is ended,
                // because the ending changes nothing of it.
                called.made("Engine.shut_down") { boundary.shutDown() }
                boundary.shutDown()
                val closed = runCatching { boundary.request("info", "{}") }.exceptionOrNull()
                require(closed is CraftException.Internal && closed.detail == CLOSED_WORDS) {
                    "a call after the end answered $closed, not the core's Internal \"$CLOSED_WORDS\""
                }
                require(boundary.recovery() == fresh) { "recovery did not answer the same after the end as it answered at the start" }
            } finally {
                // An engine a failed check left running ends here; ending it again is no error.
                boundary.shutDown()
            }

            val ids = manifest.map { it.id }
            val missed = ids.filterNot { it in called }
            require(missed.isEmpty()) {
                "the boundary now calls for ${missed.joinToString()}, which this check never makes: add each call and hold its answer"
            }
            val stale = called.filterNot { it in ids }
            require(stale.isEmpty()) { "this check makes calls the library no longer exports: ${stale.joinToString()}" }
        },
    )
