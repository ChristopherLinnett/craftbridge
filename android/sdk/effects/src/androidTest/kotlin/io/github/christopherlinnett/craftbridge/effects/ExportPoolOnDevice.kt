// The export's thread pool, sized from the memory the app itself reads as available, on a device.

package io.github.christopherlinnett.craftbridge.effects

import android.content.Context
import android.util.Log
import io.github.christopherlinnett.craftbridge.core.CraftColor
import io.github.christopherlinnett.craftbridge.core.Folders
import io.github.christopherlinnett.craftbridge.core.jsonValue
import java.io.File
import kotlin.math.sqrt

/** The sizes the pool is sized for, by the names a person gives them. */
private val frameSizes = listOf("1080p" to (1920 to 1080), "4K" to (3840 to 2160), "5K" to (5120 to 2880), "8K" to (7680 to 4320))

/**
 * The name the exporter gives its pool's threads, as the kernel keeps it: `craftbridge-export-<n>`, cut to the
 * fifteen characters a thread's name holds.
 */
private const val POOL_THREAD_NAME = "craftbridge-exp"

/** How long a check waits for the threads of an earlier engine's pool to end. */
private const val WAIT_FOR_THREADS_MS = 10_000L

/** One figure of `/proc/meminfo`, such as `MemAvailable`, as an app reads it, in bytes. */
internal fun meminfo(name: String): Long {
    val line = File("/proc/meminfo").readLines().firstOrNull { it.startsWith("$name:") } ?: error("/proc/meminfo has no $name line")
    val words = line.substringAfter(':').trim().split(Regex("\\s+"))
    require(words.size == 2 && words[1] == "kB") { "$name reads `$line`, not a figure in kB" }
    return words[0].toLong() * 1024
}

/** `MemAvailable`, the kernel's estimate of the memory an app can still allocate, which the exporter sizes its pool by. */
internal fun memAvailable(): Long = meminfo("MemAvailable")

/** The processors this process may run on, as the kernel's affinity mask lists them: the count the exporter's pool is capped at. */
private fun allowedProcessors(): Int =
    File("/proc/self/status")
        .readLines()
        .first { it.startsWith("Cpus_allowed_list:") }
        .substringAfter(':')
        .trim()
        .split(',')
        .sumOf { range ->
            val ends = range.split('-').map { it.toInt() }
            ends.last() - ends.first() + 1
        }

/**
 * The size of the export pool for [available] bytes: as many threads as they hold frames in flight of
 * `width` by `height` pixels at [perPixel] bytes a pixel — the need the exporter divides the memory by — at
 * least one and at most [cores]. Written here from the plan's rule in the plan's words, not from the
 * exporter's code, so a device compares the exporter with the rule; only the bytes a pixel come from the
 * engine, read from its own refusal ([bytesAPixelInFlight]).
 */
internal fun expectedPool(
    available: Long,
    width: Int,
    height: Int,
    cores: Int,
    perPixel: Long,
): Int = minOf(cores.toLong(), maxOf(1L, available / (width.toLong() * height * perPixel))).toInt()

/** How many threads of this process carry the pool's name. */
private fun poolThreads(): Int =
    File("/proc/self/task").listFiles().orEmpty().count { task ->
        runCatching { File(task, "comm").readText().trim() == POOL_THREAD_NAME }.getOrDefault(false)
    }

/** Waits until no thread of an earlier pool is left, since the pool a job keeps ends with its engine. */
private fun awaitNoPoolThreads() {
    val deadline = System.currentTimeMillis() + WAIT_FOR_THREADS_MS
    while (poolThreads() > 0 && System.currentTimeMillis() < deadline) {
        Thread.sleep(50)
    }
    require(poolThreads() == 0) { "${poolThreads()} threads of an earlier export pool live $WAIT_FOR_THREADS_MS ms after its engine ended" }
}

/**
 * An export of one frame of a [width] by [height] comp through a fresh engine, and how many threads the pool it ran
 * on has: the exporter's own threads where the memory held the job back, and the caller's, which are as many as the
 * cores, where it did not (so none carry the pool's name). The memory the app reads before the export starts and
 * after it has ended come with it.
 */
private suspend fun poolOfOneExport(
    context: Context,
    width: Int,
    height: Int,
): Triple<Int?, Long, Long> {
    val cache = File(Folders.appDefault(context).cache)
    val output = File(cache, "qa3b-pool.png")
    val written = { cache.listFiles().orEmpty().filter { it.name.startsWith("qa3b-pool") } }
    written().forEach { it.delete() }
    awaitNoPoolThreads()
    val engine = startOn(context)
    try {
        val comp =
            engine.idOf(
                "comp.new",
                mapOf(
                    "name" to "qa3b pool ${width}x$height".jsonValue,
                    "width" to width.toDouble().jsonValue,
                    "height" to height.toDouble().jsonValue,
                    "duration" to (1.0 / 30).jsonValue,
                    "frameRate" to 30.0.jsonValue,
                ),
                "comp",
            )
        engine.layer.newSolid(color = CraftColor.Hex("#336699"))
        val before = memAvailable()
        engine.export(
            request = mapOf("comp" to comp, "output" to output.path.jsonValue, "format" to "png".jsonValue),
            onProgress = { _, _, _ -> },
        )
        val after = memAvailable()
        require(written().isNotEmpty()) { "the ${width}x$height export wrote no frame" }
        // The pool is kept after the job for the next of its size, so it is still there to be counted.
        val named = poolThreads()
        return Triple(named.takeIf { it > 0 }, before, after)
    } finally {
        engine.shutDown()
        written().forEach { it.delete() }
    }
}

/**
 * Whether the device has the memory to hold one frame of [width] by [height] in flight at [perPixel] bytes a
 * pixel: the exporter refuses a frame it cannot hold, which leaves no pool to count, so a size the device
 * cannot hold is left out of the run and said in its log.
 */
private fun deviceHolds(
    width: Int,
    height: Int,
    perPixel: Long,
): Boolean = width.toLong() * height * perPixel <= memAvailable()

/**
 * A square size whose frame puts the rule at half the [cores] for [available] bytes at [perPixel] bytes a
 * pixel, and which the device always holds: what a device with the memory to run every named size on its
 * cores is also held to, since only a pool below the cores shows the exporter sizing it by the memory.
 */
private fun sizeBelowTheCores(
    available: Long,
    cores: Int,
    perPixel: Long,
): Pair<Int, Int> {
    val need = available / maxOf(1, cores / 2)
    val side = sqrt((need / perPixel).toDouble()).toInt()
    return side to side
}

/**
 * One export at [width] by [height], its pool held to the rule for [cores]: `null` where the device cannot hold the
 * frame, which is said in the log, and otherwise whether the rule gave fewer threads than the cores. Only there does
 * the exporter size a pool by the memory, since a pool of the cores is the one an export that ignored the memory runs
 * on too.
 */
private suspend fun poolHeldToTheRule(
    context: Context,
    name: String,
    width: Int,
    height: Int,
    cores: Int,
    perPixel: Long,
): Boolean? {
    if (!deviceHolds(width, height, perPixel)) {
        val need = width.toLong() * height * perPixel
        Log.i(LOG_TAG, "pool at $name: left out, ${memAvailable()} bytes are available for a frame in flight needing $need")
        return null
    }
    val (named, before, after) = poolOfOneExport(context, width, height)
    // A pool of as many threads as the cores is the caller's own, which carries no name of the pool's.
    val observed = named ?: cores
    val least = expectedPool(minOf(before, after), width, height, cores, perPixel)
    val most = expectedPool(maxOf(before, after), width, height, cores, perPixel)
    val said = "MemAvailable $before before and $after after, $cores cores: the rule gives $least to $most"
    Log.i(LOG_TAG, "pool at $name: $observed threads, $said")
    require(observed in least..most) { "the export at $name ran on a pool of $observed threads; $said" }
    return most < cores
}

/**
 * The checks of the export pool's size (the plan's AD7) inside an app: the memory figure the exporter sizes by is
 * the one an app can read, and at each size the pool is the one the rule gives for it.
 */
internal val exportPoolChecks: List<Case> =
    listOf(
        Case("an app reads MemAvailable from /proc/meminfo, and a figure below the phone's memory") {
            val available = memAvailable()
            val total = meminfo("MemTotal")
            Log.i(LOG_TAG, "MemAvailable $available bytes of $total")
            require(available in 1 until total) { "MemAvailable $available is not a figure below MemTotal $total" }
        },
        Case(
            "at 1080p, 4K, 5K and 8K the export's pool is min(cores, max(1, MemAvailable / (width x height x the bytes a pixel " +
                "in flight the exporter's refusal counts)))",
        ) { context ->
            val cores = allowedProcessors()
            val probe = startOn(context)
            val perPixel =
                try {
                    probe.bytesAPixelInFlight(context)
                } finally {
                    probe.shutDown()
                }
            Log.i(LOG_TAG, "pool: the exporter counts $perPixel bytes a pixel in flight")
            val held =
                frameSizes.mapNotNull { (name, size) ->
                    poolHeldToTheRule(context, name, size.first, size.second, cores, perPixel)?.let { below -> name to below }
                }
            var sized = held.filter { it.second }.map { it.first }
            if (sized.isEmpty()) {
                val (width, height) = sizeBelowTheCores(memAvailable(), cores, perPixel)
                val name = "${width}x$height"
                sized = listOfNotNull(name.takeIf { poolHeldToTheRule(context, name, width, height, cores, perPixel) == true })
            }
            require(sized.isNotEmpty()) {
                val sizes = frameSizes.map { it.first }
                "at none of the sizes the device holds, ${held.map { it.first }} of $sizes, nor at one of its own, does the rule give " +
                    "fewer threads than its $cores cores, so no export showed a pool sized by the memory"
            }
        },
    )
