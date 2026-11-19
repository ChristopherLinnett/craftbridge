// An export whose frame the phone's memory cannot hold is refused before it renders, in words that name
// the frame and the memory, and the phone goes on exporting frames it can hold. The engine reads the
// kernel's own figure, so the checks ask for frames that figure cannot hold: the largest comp EffectCraft
// makes, which no device holds, and an 8K comp where the device's free memory sits, or can be brought,
// under its need. The exporter's own count of what a frame in flight needs is read from its refusal, so no
// Kotlin number stands for it.

package io.github.christopherlinnett.craftbridge.effects

import android.content.Context
import android.graphics.Bitmap
import android.util.Log
import io.github.christopherlinnett.craftbridge.core.CommandFailedException
import io.github.christopherlinnett.craftbridge.core.Folders
import io.github.christopherlinnett.craftbridge.core.jsonValue
import java.io.File

/** The side of the largest comp EffectCraft makes (its `MAX_SIZE`). One copy of its frame alone is 14.4 GB at
 * the compositor's four `f32` channels a pixel, more than any device these checks run on has free, so its
 * export is refused before it renders on every one of them. */
private const val LARGEST_SIDE = 30_000

/** The pixels of an 8K frame. */
private const val PIXELS_8K = 7680L * 4320L

/** How far the kernel's figure may drift on a working phone over one export's time, or while held memory
 * is given back. */
private const val DRIFT = 128L shl 20

/** One ARGB_8888 picture of the frame's pixels: native memory the kernel counts against its free
 * figure, filled so every page is real, and gives back the moment the check recycles it. */
private const val HELD_PICTURE = PIXELS_8K * 4L

/** The picture buffers one check holds at most: enough to bring a two-gigabyte emulator under one frame's
 * need, few enough that the run never takes what a device had for its own working set. A device idling
 * with more free memory than these cover can hold the frame; there the 8K refusal has nothing to refuse,
 * and the check says so instead of failing a device that is simply well off. */
private const val MOST_HELD_PICTURES = 12

/** The refusal's words, as the exporter writes them for a frame rendered on the CPU, which these checks'
 * engines render on. */
private val refusalWords =
    Regex("a frame of (\\d+)x(\\d+) needs (\\d+) bytes to render on the CPU and write, and the device reports (\\d+) bytes available")

/** What the exporter said when it refused a frame: its size, what one frame of it in flight needs, and the
 * memory the device reported, each read from the refusal's own words. */
internal class Refusal(
    val width: Int,
    val height: Int,
    val need: Long,
    val available: Long,
)

/** An export of a [width] by [height] comp carrying a whole-frame fill, which the engine must refuse: an
 * export of it would allocate a real frame's pixels, so the refusal is the guard standing between the phone
 * and the memory an unguarded export would be ended for wanting. The refusal is read back from its words and
 * held to the device: its figure is the kernel's live one, within [DRIFT] (a figure fixed in the SDK's own
 * words would sit nowhere near it), and under the need it names; the export left no file in the cache, and
 * no record of itself in the engine's config folder, where the list of files an export may write is kept. */
internal suspend fun EffectsEngine.refusalOf(
    context: Context,
    width: Int,
    height: Int,
    name: String,
): Refusal {
    val folders = Folders.appDefault(context)
    val cache = File(folders.cache)
    deleteCacheFiles(cache, name)
    val comp =
        idOf(
            "comp.new",
            mapOf(
                "name" to name.jsonValue,
                "width" to width.toDouble().jsonValue,
                "height" to height.toDouble().jsonValue,
                "duration" to 1.0.jsonValue,
                "frameRate" to 30.0.jsonValue,
            ),
            "comp",
        )
    idOf("layer.newSolid", mapOf("comp" to comp, "name" to "$name fill".jsonValue), "layer")
    val failure =
        try {
            export(
                request = mapOf("comp" to comp, "output" to File(cache, "$name.png").path.jsonValue, "format" to "png".jsonValue),
                onProgress = { _, _, _ -> },
            )
            error("the ${width}x$height export ran where one frame in flight has no room")
        } catch (failed: CommandFailedException) {
            failed
        }
    val words = failure.message ?: error("the refused export answered no words")
    Log.i(LOG_TAG, "fx1v: the refusal: $words")
    val said = refusalWords.find(words)?.groupValues ?: error("the refusal is not in the exporter's words: $words")
    val refusal = Refusal(said.elementAt(1).toInt(), said.elementAt(2).toInt(), said.elementAt(3).toLong(), said.elementAt(4).toLong())
    require(refusal.width == width && refusal.height == height) { "the refusal names another frame: $words" }
    require(refusal.available < refusal.need) { "the refusal names a figure that holds the frame: $words" }
    val live = memAvailable()
    val drift = if (refusal.available > live) refusal.available - live else live - refusal.available
    require(drift <= DRIFT) { "the refusal figures ${refusal.available} bytes, but the kernel reports $live bytes: $words" }
    require(cacheFilesStartingWith(cache, name).isEmpty()) { "the refused export wrote a file: ${cacheFilesStartingWith(cache, name)}" }
    val records =
        File(folders.config)
            .listFiles()
            ?.filter { file -> file.isFile && file.readText().contains(name) }
            .orEmpty()
    require(records.isEmpty()) { "the refused export is still listed in $records" }
    return refusal
}

/** The bytes a pixel the exporter counts one frame in flight at on the CPU, read from its own refusal of the
 * largest comp: the frame's bytes times the copies the exporter holds of it, which the pool's size divides
 * the memory by and the refusal compares with it. Read from the engine, not spelt again here, so a change to
 * the exporter's count reaches every check with the engine that carries it. */
internal suspend fun EffectsEngine.bytesAPixelInFlight(context: Context): Long {
    val pixels = LARGEST_SIDE.toLong() * LARGEST_SIDE
    val refusal = refusalOf(context, LARGEST_SIDE, LARGEST_SIDE, "fx1v-largest")
    require(refusal.need % pixels == 0L) { "a need of ${refusal.need} bytes is no whole number of bytes for each of its $pixels pixels" }
    return refusal.need / pixels
}

/** Takes the phone's memory in native pixel buffers until the kernel reports less free than [need] — the
 * state in which an export of a frame needing that much must answer the refusal. Pages the kernel has never
 * seen are no memory it misses, so every buffer is written full, the way a rendered frame writes its own.
 * Answers false, holding nothing, where the device idles with more free memory than [MOST_HELD_PICTURES]
 * buffers can take; where it took what it could and the figure stayed above the need, that is an error in
 * its own words, since the check would else trust a refusal it cannot place. The figure is aimed two buffers
 * under the need, room to spare over the drift bound. */
private fun driveMemoryUnder(
    need: Long,
    held: MutableList<Bitmap>,
): Boolean {
    val want = ((memAvailable() - (need - 2 * HELD_PICTURE)) / HELD_PICTURE + 1).toInt()
    if (want > MOST_HELD_PICTURES) return false
    repeat(want) {
        val picture =
            try {
                Bitmap.createBitmap(7680, 4320, Bitmap.Config.ARGB_8888)
            } catch (refused: OutOfMemoryError) {
                error("the phone would not give up any more memory to hold: ${refused.message}")
            }
        picture.eraseColor(0xFF5A5A5A.toInt())
        held += picture
        if (memAvailable() < need) return true
    }
    val now = memAvailable()
    if (now < need) return true
    error(
        "the phone took ${held.size.toLong() * HELD_PICTURE} bytes and still reports $now free, above the $need one 8K frame in flight needs",
    )
}

/** Gives [held] back: a recycled buffer's pages belong to the kernel again at once, so the check
 * recycles every picture and waits for the kernel's figure to climb back to within [DRIFT] of the [before]
 * it had before the hold, saying so in words when it does not — later checks share this process, and
 * must not share its held memory too. Where nothing was held there is nothing to wait for. */
private fun releaseMemory(
    held: MutableList<Bitmap>,
    before: Long,
) {
    val took = held.size
    held.forEach { picture -> picture.recycle() }
    held.clear()
    if (took == 0) return
    repeat(150) {
        if (memAvailable() >= before - DRIFT) return
        Thread.sleep(100)
    }
    error("the held memory was not given back: MemAvailable is still ${memAvailable()} bytes, of $before before it was held")
}

/** The bytes a PNG starts with, which is how a check knows the file it was handed is a rendered frame. */
private val pngStart = byteArrayOf(0x89.toByte(), 'P'.code.toByte(), 'N'.code.toByte(), 'G'.code.toByte())

/** The cache's files whose names start with [prefix]. */
private fun cacheFilesStartingWith(
    cache: File,
    prefix: String,
): List<File> =
    cache
        .listFiles()
        ?.filterNotNull()
        ?.filter { file -> file.name.startsWith(prefix) }
        .orEmpty()

/** Takes away the cache's files whose names start with [prefix]: what a check wrote, its finally deletes. */
private fun deleteCacheFiles(
    cache: File,
    prefix: String,
) {
    cacheFilesStartingWith(cache, prefix).forEach { it.delete() }
}

private fun guardChecksOnDevice(): List<Case> =
    listOf(
        Case(
            "an export whose frame cannot fit the phone's free memory is refused in words before any file, and the engine goes on exporting",
        ) { context ->
            val cache = File(Folders.appDefault(context).cache)
            deleteCacheFiles(cache, "fx1v-after")
            val freeBefore = memAvailable()
            val held = mutableListOf<Bitmap>()
            val engine = startOn(context)
            try {
                // On every device: no device holds a frame of the largest comp.
                val need8K = engine.bytesAPixelInFlight(context) * PIXELS_8K
                if (driveMemoryUnder(need8K, held)) {
                    Log.i(LOG_TAG, "fx1v: MemAvailable was $freeBefore, now ${memAvailable()} bytes; one 8K frame in flight needs $need8K")
                    val refused = engine.refusalOf(context, 7680, 4320, "fx1v-8k")
                    require(refused.need == need8K) {
                        "an 8K frame in flight needs ${refused.need} bytes, where the largest comp's refusal counts $need8K for its pixels"
                    }
                } else {
                    Log.i(
                        LOG_TAG,
                        "fx1v: the device idles with $freeBefore bytes free, more than $MOST_HELD_PICTURES held frames can bring under " +
                            "one 8K frame's need ($need8K): the frame fits this device, so its refusal is not shown here; the " +
                            "largest comp's is",
                    )
                }
                // The refusals ended their exports, not the app: on the same engine, in the same held memory, a
                // frame the phone can hold still exports.
                val small =
                    engine.idOf(
                        "comp.new",
                        mapOf(
                            "name" to "fx1v small".jsonValue,
                            "width" to 64.0.jsonValue,
                            "height" to 64.0.jsonValue,
                            "duration" to 1.0.jsonValue,
                            "frameRate" to 30.0.jsonValue,
                        ),
                        "comp",
                    )
                engine.export(
                    request =
                        mapOf(
                            "comp" to small,
                            "output" to File(cache, "fx1v-after.png").path.jsonValue,
                            "format" to "png".jsonValue,
                        ),
                    onProgress = { _, _, _ -> },
                )
                val made = cacheFilesStartingWith(cache, "fx1v-after")
                require(made.isNotEmpty()) { "the export after the refused one wrote nothing" }
                require(
                    made
                        .first()
                        .readBytes()
                        .copyOfRange(0, pngStart.size)
                        .contentEquals(pngStart),
                ) {
                    "the export after the refused one wrote no PNG frame: $made"
                }
            } finally {
                engine.shutDown()
                releaseMemory(held, freeBefore)
                deleteCacheFiles(cache, "fx1v-after")
            }
        },
        Case("a 1080p export finishes on the phone, its frames all written") { context ->
            val cache = File(Folders.appDefault(context).cache)
            deleteCacheFiles(cache, "fx1v-hd")
            val engine = startOn(context)
            try {
                val comp =
                    engine.idOf(
                        "comp.new",
                        mapOf(
                            "name" to "fx1v 1080p".jsonValue,
                            "width" to 1920.0.jsonValue,
                            "height" to 1080.0.jsonValue,
                            "duration" to 1.0.jsonValue,
                            "frameRate" to 30.0.jsonValue,
                        ),
                        "comp",
                    )
                // A whole-frame fill, as the refused comps carry: every frame is a real frame's pixels.
                engine.idOf("layer.newSolid", mapOf("comp" to comp, "name" to "fx1v-hd fill".jsonValue), "layer")
                val heard = mutableListOf<Pair<ULong, ULong>>()
                engine.export(
                    request = mapOf("comp" to comp, "output" to File(cache, "fx1v-hd.png").path.jsonValue, "format" to "png".jsonValue),
                    onProgress = { done, total, _ -> heard += done to total },
                )
                require(heard.isNotEmpty()) { "the 1080p export reported no progress at all" }
                val made = cacheFilesStartingWith(cache, "fx1v-hd")
                require(made.isNotEmpty()) { "the 1080p export wrote nothing" }
                require(
                    made
                        .first()
                        .readBytes()
                        .copyOfRange(0, pngStart.size)
                        .contentEquals(pngStart),
                ) { "the 1080p export wrote no PNG frame: $made" }
                Log.i(LOG_TAG, "fx1v: a 1080p export finished with ${made.size} files at MemAvailable ${memAvailable()} bytes")
            } finally {
                engine.shutDown()
                deleteCacheFiles(cache, "fx1v-hd")
            }
        },
    )

/** Wired into the run in `CraftBridgeInstrumentation`, after the plugin's own policy checks. */
internal val memoryGuardChecks: List<Case> = guardChecksOnDevice()
