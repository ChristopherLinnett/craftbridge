// A failed or stopped export leaves no output on a device but the renders that finished: a file at an output
// path always means a finished render. The render fails part way, after it wrote pictures of its own, over the
// engine's real exporter, and what is left at the output is only what stood there before the export began, and
// the movie of an item of its queue that finished before the one that failed or was stopped.

package io.github.christopherlinnett.craftbridge.effects

import android.content.Context
import io.github.christopherlinnett.craftbridge.core.CancelledException
import io.github.christopherlinnett.craftbridge.core.CommandFailedException
import io.github.christopherlinnett.craftbridge.core.CraftColor
import io.github.christopherlinnett.craftbridge.core.CraftException
import io.github.christopherlinnett.craftbridge.core.Folders
import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.core.jsonValue
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicReference

/** The picture of the sequence the render cannot write: a read-only file stands at its name, so creating it fails there. */
private const val BLOCKED_PICTURE = "frame_00150.png"

/** What stood at [BLOCKED_PICTURE] before the export, which a failed export must leave as it was. */
private const val EARLIER_PICTURE = "an earlier picture"

/** The movie of the first item of a queue, which finishes before the second item fails or is stopped. */
private const val FIRST_MOVIE = "first.mp4"

/**
 * Runs [check] on an engine holding a composition of ten pictures a second for twenty seconds, big enough that a
 * render is still writing when the export reports, so the pictures it wrote are there to be seen as it fails, and
 * on the folder `into` holding the read-only file at [BLOCKED_PICTURE]. A scene that is not [blocking] has no such
 * file and lasts [seconds] seconds, so a sequence of it renders on for as long as a stop needs to reach it. The
 * folder is removed when it ends.
 */
private suspend fun failingScene(
    context: Context,
    name: String,
    seconds: Double = 20.0,
    blocking: Boolean = true,
    check: suspend (engine: EffectsEngine, into: File, comp: JSONValue) -> Unit,
) {
    val into = File(Folders.appDefault(context).cache, name)
    into.deleteRecursively()
    into.mkdirs()
    val blocked = File(into, BLOCKED_PICTURE)
    if (blocking) {
        blocked.writeText(EARLIER_PICTURE)
        require(blocked.setReadOnly()) { "the picture at $blocked could not be made read-only" }
        // A user the permissions do not hold back can write the picture, and no render fails.
        require(!blocked.canWrite()) { "the read-only picture at $blocked is still writable, so the render cannot fail there" }
    }
    val engine = startOn(context)
    try {
        val comp =
            engine.idOf(
                "comp.new",
                mapOf(
                    "name" to name.jsonValue,
                    "width" to 1280.0.jsonValue,
                    "height" to 720.0.jsonValue,
                    "duration" to seconds.jsonValue,
                    "frameRate" to 10.0.jsonValue,
                ),
                "comp",
            )
        engine.layer.newSolid(color = CraftColor.Hex("#336699"))
        check(engine, into, comp)
        require(!blocking || blocked.readText() == EARLIER_PICTURE) { "the file that was there changed" }
    } finally {
        engine.shutDown()
        blocked.setWritable(true)
        into.deleteRecursively()
    }
}

/** The request that renders `comp` as a sequence of pictures into [into], which fails at [BLOCKED_PICTURE]. */
private fun sequenceRequest(
    comp: JSONValue,
    into: File,
): Map<String, JSONValue> = mapOf("comp" to comp, "output" to File(into, "frame.png").path.jsonValue, "format" to "png".jsonValue)

/** Queues a movie of one second, which finishes, then the sequence the render cannot finish, over [comp]. */
private suspend fun EffectsEngine.queueMovieThenSequence(
    comp: JSONValue,
    into: File,
) {
    exec(
        "renderQueue.add",
        mapOf(
            "format" to "h264".jsonValue,
            "output" to File(into, FIRST_MOVIE).path.jsonValue,
            "end" to 1.0.jsonValue,
            "resolution" to "quarter".jsonValue,
            "comp" to comp,
        ),
    )
    exec("renderQueue.add", sequenceRequest(comp, into) + mapOf("resolution" to "half".jsonValue))
}

/** The failure the export of the project's whole queue throws, or null when it finishes. */
private suspend fun EffectsEngine.failureOfQueue(): CraftException? =
    try {
        export(request = mapOf("queue" to true.jsonValue), onProgress = { _, _, _ -> })
        null
    } catch (failed: CommandFailedException) {
        failed
    }

/** Wired into the run in `CraftBridgeInstrumentation`, after the memory guard's checks. */
internal val failedExportChecks: List<Case> =
    listOf(
        Case("an export that fails part way leaves nothing but what was already there") { context ->
            failingScene(context, "fx-failed") { engine, into, comp ->
                val seen = ConcurrentHashMap.newKeySet<String>()
                val failure =
                    try {
                        engine.export(
                            request = sequenceRequest(comp, into),
                            onProgress = { _, _, _ -> seen += into.list().orEmpty() },
                        )
                        error("the export finished although the picture it needed stood read-only")
                    } catch (failed: CommandFailedException) {
                        failed
                    }
                say("the render failed: ${failure.message}")
                require(seen.any { it != BLOCKED_PICTURE }) { "the render had written nothing before it failed, so it proves nothing" }
                val left = into.list().orEmpty().toList()
                require(left == listOf(BLOCKED_PICTURE)) { "the failed export left a file of its own: $left" }
            }
        },
        Case("a queue that fails keeps the movie of the item that finished") { context ->
            failingScene(context, "fx-queue-failed") { engine, into, comp ->
                engine.queueMovieThenSequence(comp, into)
                val failure = engine.failureOfQueue()
                require(failure != null) { "the queue finished although the picture it needed stood read-only" }
                say("the queue failed: ${failure.message}")
                val left = into.list().orEmpty().toSortedSet()
                require(left == sortedSetOf(BLOCKED_PICTURE, FIRST_MOVIE)) {
                    "a failed queue lost the movie that finished, or left a file of the item that failed: $left"
                }
                require(File(into, FIRST_MOVIE).length() > 0) { "the finished item's movie is empty" }
            }
        },
        Case("a queue that is stopped keeps the movie of the item that finished") { context ->
            failingScene(context, "fx-queue-stopped", seconds = 600.0, blocking = false) { engine, into, comp ->
                engine.queueMovieThenSequence(comp, into)
                val lock = Any()
                val ended = AtomicReference<CraftException?>(null)
                coroutineScope {
                    synchronized(lock) {
                        lateinit var call: Job
                        call =
                            launch(start = CoroutineStart.LAZY) {
                                ended.set(
                                    try {
                                        engine.export(
                                            request = mapOf("queue" to true.jsonValue),
                                            onProgress = { _, _, _ ->
                                                synchronized(lock) {
                                                    if (into.list().orEmpty().any { it.startsWith("frame_") }) call.cancel()
                                                }
                                            },
                                        )
                                        null
                                    } catch (stopped: CancellationException) {
                                        stopped.cause as? CraftException ?: throw stopped
                                    },
                                )
                            }
                        call.start()
                    }
                }
                val failure = ended.get()
                require(failure is CancelledException) { "the stop was expected to end as Cancelled, not as $failure" }
                val left = into.list().orEmpty().toSortedSet()
                require(left == sortedSetOf(FIRST_MOVIE)) {
                    "a stopped queue lost the movie that finished, or left a file of the item it stopped: $left"
                }
                require(File(into, FIRST_MOVIE).length() > 0) { "the finished item's movie is empty" }
            }
        },
    )
