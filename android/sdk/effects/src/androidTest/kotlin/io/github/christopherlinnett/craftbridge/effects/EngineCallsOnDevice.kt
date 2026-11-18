// The engine's calls over the real boundary: progress to a lambda, and a coroutine cancelled.

package io.github.christopherlinnett.craftbridge.effects

import android.content.Context
import io.github.christopherlinnett.craftbridge.core.CancelledException
import io.github.christopherlinnett.craftbridge.core.Engine
import io.github.christopherlinnett.craftbridge.core.Folders
import io.github.christopherlinnett.craftbridge.core.UnsupportedException
import io.github.christopherlinnett.craftbridge.ffi.core.GpuChoice
import io.github.christopherlinnett.craftbridge.ffi.effects.startEffectsEngine
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.cancel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import java.io.File

/**
 * The frames of the comp an export renders: small enough to render quickly on a phone's CPU, long
 * enough for the engine to report its progress more than once on the way through.
 */
private const val EXPORT_FRAMES = 300

/**
 * The frames of the comp a cancel is caught inside: small frames render at a good pace on a phone, so
 * these are needed to keep the export running long enough for a beat to arrive while it is still going
 * and for the engine to see the stop at the beat after that.
 */
private const val CANCEL_FRAMES = 3_000

/** How long a check waits for an export to report itself rendering before giving up on it. */
private const val WAIT_FOR_RENDER = 120_000L

/**
 * The engine every engine check starts, through the SDK's own start, which hands the device's codec list over
 * first, and ended again however the checks it runs end.
 */
internal suspend fun onEngine(
    context: Context,
    gpu: GpuChoice = GpuChoice.CPU_ONLY,
    check: suspend (Engine) -> Unit,
) {
    val engine = Engine.start(Folders.appDefault(context)) { startEffectsEngine(it, gpu) }
    try {
        check(engine)
    } finally {
        engine.shutDown()
    }
}

internal val engineChecks: List<Case> =
    listOf(
        Case("a method that reports progress is refused by a plain request") { context ->
            onEngine(context) { engine ->
                val refused = runCatching { engine.request("export") }.exceptionOrNull()
                require(
                    refused is UnsupportedException,
                ) { "`export` over a plain request was answered $refused, not refused as unsupported" }
                require(refused.message.orEmpty().contains("export")) { "the refusal does not name what it refuses: ${refused.message}" }
                require(refused.message == refused.detail) { "the message ${refused.message} is not the engine's words ${refused.detail}" }
            }
        },
        Case("an export reports its progress to the lambda and writes its frames") { context ->
            val output = exportInto(context, "kotlin-export.png")
            onEngine(context) { engine ->
                engine.request("run", compRequest(EXPORT_FRAMES))
                val heard = mutableListOf<Pair<ULong, ULong>>()
                engine.requestWithProgress("export", exportRequest(output)) { done, total, _ -> heard += done to total }
                require(heard.isNotEmpty()) { "the export reported no progress at all" }
                require(heard.last().second == EXPORT_FRAMES.toULong()) {
                    "the last report did not count the comp's $EXPORT_FRAMES frames: $heard"
                }
                require(
                    heard.zipWithNext().all { (before, after) -> after.first >= before.first && after.second >= before.second },
                ) { "progress went backwards: $heard" }
                require(heard.all { it.second == 0uL || it.first <= it.second }) { "a report counted more frames than its total: $heard" }
                val written = (0 until EXPORT_FRAMES).count { File(output.parentFile, frameName(output, it)).isFile }
                require(written == EXPORT_FRAMES) { "the export wrote $written of $EXPORT_FRAMES frames" }
            }
            removeExports(output, "kotlin-export")
        },
        Case("a coroutine cancelled mid-export ends cancelled by the engine's stop and leaves nothing behind") { context ->
            val output = exportInto(context, "kotlin-cancel.png")
            removeExports(output, "kotlin-cancel")
            onEngine(context) { engine ->
                engine.request("run", compRequest(CANCEL_FRAMES))
                val heard = mutableListOf<Pair<ULong, ULong>>()
                val rendering = CompletableDeferred<Unit>()
                var answered: Throwable? = null
                coroutineScope {
                    val export =
                        launch {
                            answered =
                                runCatching {
                                    engine.requestWithProgress("export", exportRequest(output)) { done, total, _ ->
                                        heard += done to total
                                        // Only a report that counts frames already written says the render is in
                                        // progress: cancelling on the first beat of all could land before it starts.
                                        if (total > 0uL && done < total) {
                                            rendering.complete(Unit)
                                        }
                                    }
                                }.exceptionOrNull()
                        }
                    if (withTimeoutOrNull(WAIT_FOR_RENDER) { rendering.await() } != null) {
                        // Twice: a second cancel while the engine winds down must change nothing.
                        export.cancel()
                        export.cancel()
                    }
                }
                require(endedByTheEnginesStop(answered)) { "the cancelled export ended $answered, not as a cancel from its stop: $heard" }
                val left = exportsNamed(output, "kotlin-cancel")
                require(left.isEmpty()) { "the cancelled export left ${left.map { it.name }}, so it never stopped" }
            }
        },
        Case("a coroutine cancelled before it asks for an export ends cancelled by the engine's stop and writes nothing") { context ->
            val output = exportInto(context, "kotlin-precancel.png")
            removeExports(output, "kotlin-precancel")
            onEngine(context) { engine ->
                engine.request("run", compRequest(EXPORT_FRAMES))
                var answered: Throwable? = null
                coroutineScope {
                    launch {
                        // Cancelled from inside, so the cancel has landed before the call is made, as one from
                        // another thread can land at any moment before it.
                        cancel()
                        answered =
                            runCatching { engine.requestWithProgress("export", exportRequest(output)) { _, _, _ -> } }.exceptionOrNull()
                    }
                }
                require(endedByTheEnginesStop(answered)) { "a cancelled coroutine's export ended $answered, not as a cancel from its stop" }
                val left = exportsNamed(output, "kotlin-precancel")
                require(left.isEmpty()) { "an export asked for by a cancelled coroutine wrote ${left.size} files, so nothing stopped it" }
            }
        },
    )

/**
 * Whether a call ended as its cancelled coroutine does, with the engine's own answer to the stop as the
 * reason, in the core's word for it: proof both that the stop reached the engine and that the call waited
 * for it to wind down.
 */
private fun endedByTheEnginesStop(answered: Throwable?): Boolean {
    val stop = (answered as? CancellationException)?.cause
    return stop is CancelledException && stop.message == "cancelled"
}

/** An output of the engine's own, in the folder the system may empty, which is where a check's files go. */
private fun exportInto(
    context: Context,
    name: String,
): File = File(Folders.appDefault(context).cache, name)

/** A comp of the given frames at the engine's frame rate, small enough for a phone's CPU. */
private fun compRequest(frames: Int): String =
    """{"steps":[{"command":"comp.new","params":{"width":640,"height":360,"duration":${frames / 30.0},"frameRate":30}}]}"""

/** An export of the comp to `output` as a sequence of images. */
private fun exportRequest(output: File): String = """{"output":"${output.path}","format":"png"}"""

/** The name the exporter gives frame `frame` of the sequence asked for at `output`. */
private fun frameName(
    output: File,
    frame: Int,
): String = output.nameWithoutExtension + "_" + frame.toString().padStart(5, '0') + ".png"

/** The files already written for the sequence asked for at `output`. */
private fun exportsNamed(
    output: File,
    prefix: String,
): List<File> =
    output.parentFile
        ?.listFiles()
        ?.filter { it.name.startsWith(prefix) }
        .orEmpty()

private fun removeExports(
    output: File,
    prefix: String,
) {
    for (written in exportsNamed(output, prefix)) {
        written.delete()
    }
}
