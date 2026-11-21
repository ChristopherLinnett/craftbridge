// The baseline of the running example on a phone (plan §9, §29, V5): how long its first preview and its full
// export take through the SDK as an app calls it, the part of the first preview that is the GPU's start and the path
// of a preview on a warm engine, the most memory the process held, the threads the export ran on and what the phone
// was doing, said on one line a script reads off by the names `conformance/effects/baseline.toml` gives, which the
// Swift test reads too. The figures are a baseline only when the check ran alone (`-e only` the measuring check's
// name) in a run of its own on the acceptance phone.

package io.github.christopherlinnett.craftbridge.effects

import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.MediaMetadataRetriever
import android.os.BatteryManager
import android.os.Handler
import android.os.HandlerThread
import android.os.PowerManager
import android.os.SystemClock
import android.view.Surface
import io.github.christopherlinnett.craftbridge.core.Folders
import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.core.jsonValue
import io.github.christopherlinnett.craftbridge.ffi.core.GpuChoice
import java.io.File
import java.util.concurrent.Semaphore
import java.util.concurrent.TimeUnit
import kotlin.math.abs
import kotlin.time.Duration.Companion.seconds
import kotlin.time.TimeSource

/** The baseline file in the test APK's assets, beside the corpus. */
private const val BASELINE_ASSET = "baseline.toml"

/** The platform this run measures. */
private const val PLATFORM = "android"

/** The platforms the baseline is measured on. */
private val PLATFORMS = listOf("ios", "android")

/** The keys of a figure's table. */
private val FIGURE_WORDS = setOf("name", "decimals", "flagged", "platforms", "unheld_on", "condition", "condition_bound", "what")

/** What a figure that is a run's condition asks of a run for a release to be compared on it. */
internal sealed class BaselineCondition {
    /** The run reads the value the record's own runs read. */
    data object Record : BaselineCondition()

    /** The run reads no more than [bound], which the file states once for every record. */
    data class AtMost(
        val bound: Double,
    ) : BaselineCondition()
}

/**
 * One figure of the line: its name, the decimals its value is said with, whether a release whose figure is more
 * than the baseline's allowance above it is flagged, which a run's condition is not, the platforms that
 * measure it, those among them whose runs are reported but never flagged for their own noise, and, for a figure
 * that is a condition, what it asks of a run.
 */
internal class BaselineFigure(
    val name: String,
    val decimals: Int,
    val flagged: Boolean,
    val platforms: List<String>,
    val unheldOn: List<String>,
    val condition: BaselineCondition?,
)

/** The running example of plan §5, which every run builds, previews and exports. */
internal class BaselineExample(
    val width: Int,
    val height: Int,
    val seconds: Int,
    val frameRate: Int,
    val frames: Int,
    val format: String,
    val previewAtSeconds: Double,
    val previewRepeats: Int,
)

/** What a run measures and says, as `baseline.toml` defines it. */
internal class Baseline(
    /** The word the line starts with. */
    val line: String,
    /** The word the run's warm preview repeats are said under, after the figures. */
    val repeatsWord: String,
    /** The decimals each warm preview repeat is said with. */
    val repeatsDecimals: Int,
    val example: BaselineExample,
    /** The figures, in the order the line says them. */
    val figures: List<BaselineFigure>,
) {
    /** The figures a run on Android measures, in the order the line says them. */
    val measured: List<BaselineFigure> get() = figures.filter { PLATFORM in it.platforms }

    /**
     * The line a run says for [values], one for each figure it measures by name and no other, in the order the file
     * gives, then every one of the run's warm preview [repeats] in the order they ran.
     */
    fun line(
        values: Map<String, Double>,
        repeats: List<Double>,
    ): String {
        require(values.keys == measured.map { it.name }.toSet()) {
            "the run measured ${values.keys.sorted()}, and the baseline names ${measured.map { it.name }}"
        }
        require(repeats.size == example.previewRepeats) {
            "the run made ${repeats.size} warm previews, and the running example makes ${example.previewRepeats}"
        }

        fun said(
            value: Double,
            decimals: Int,
        ) = "%.${decimals}f".format(java.util.Locale.ROOT, value)
        return (
            listOf(line) + measured.map { "${it.name}=${said(values.getValue(it.name), it.decimals)}" } +
                "$repeatsWord=${repeats.joinToString(",") { said(it, repeatsDecimals) }}"
        ).joinToString(" ")
    }

    companion object {
        /** The baseline in the test APK's assets. */
        fun of(context: Context): Baseline = of(assetText(context, BASELINE_ASSET))

        /**
         * What the figure at [pointer] says of being a run's condition: `condition = "record"` asks a run to read what
         * the record's runs read, `condition = "at_most"` with a `condition_bound` asks it to read no more than the
         * bound, and a figure with neither is a cost or a plain report. A condition is never a cost, which `flagged`
         * says of the costs.
         */
        private fun condition(
            document: JSONValue,
            pointer: String,
            name: String,
            flagged: Boolean,
        ): BaselineCondition? {
            val kind = document.valueAt("$pointer/condition")
            val bound = document.valueAt("$pointer/condition_bound")
            if (kind == null) {
                if (bound != null) throw RunnerFailure("the figure `$name` holds a `condition_bound` and no `condition`")
                return null
            }
            if (flagged) throw RunnerFailure("the figure `$name` is a condition and flagged, which only a cost is")
            val word = (kind as? JSONValue.Text)?.value
            return when (word) {
                "record" -> {
                    if (bound != null) {
                        throw RunnerFailure("the figure `$name` is `record` and holds a `condition_bound`, which it has no use for")
                    }
                    BaselineCondition.Record
                }

                "at_most" -> {
                    BaselineCondition.AtMost(
                        (bound as? JSONValue.Number)?.value
                            ?: throw RunnerFailure("the figure `$name` is `at_most` and names no number `condition_bound`"),
                    )
                }

                else -> {
                    val said = word?.let { "`$it`" } ?: "not text"
                    throw RunnerFailure("the figure `$name`'s `condition` is $said, which is neither record nor at_most")
                }
            }
        }

        /** The baseline [text] holds. */
        fun of(text: String): Baseline {
            val document = tomlDocument(text, "the baseline")

            fun number(pointer: String) =
                (document.valueAt(pointer) as? JSONValue.Number)?.value ?: throw RunnerFailure("the baseline has no number at $pointer")

            fun text(pointer: String) =
                (document.valueAt(pointer) as? JSONValue.Text)?.value ?: throw RunnerFailure("the baseline has no text at $pointer")

            fun flag(pointer: String) =
                (document.valueAt(pointer) as? JSONValue.Bool)?.value
                    ?: throw RunnerFailure("the baseline has no true or false at $pointer")
            val example =
                BaselineExample(
                    number("/running_example/width").toInt(),
                    number("/running_example/height").toInt(),
                    number("/running_example/seconds").toInt(),
                    number("/running_example/frame_rate").toInt(),
                    number("/running_example/frames").toInt(),
                    text("/running_example/format"),
                    number("/running_example/preview_at_seconds"),
                    number("/running_example/preview_repeats").toInt(),
                )
            val listed = (document.valueAt("/figure") as? JSONValue.Array)?.values.orEmpty()
            if (listed.isEmpty()) throw RunnerFailure("the baseline defines no figure")
            return Baseline(
                text("/line"),
                text("/repeats_word"),
                number("/repeats_decimals").toInt(),
                example,
                listed.indices.map {
                    val name = text("/figure/$it/name")
                    // A key spelt otherwise, `unheld_on` above all, would be read as absent: a figure meant to be
                    // reported only would be flagged on every release.
                    val keys = (document.valueAt("/figure/$it") as? JSONValue.Object)?.entries?.keys.orEmpty()
                    keys.filter { key -> key !in FIGURE_WORDS }.minOrNull()?.let { key ->
                        throw RunnerFailure("the figure `$name` holds the key `$key`, which no figure has")
                    }

                    // A platform spelt otherwise would leave the figure out of every run's line and every record
                    // unseen, or flag it where it is to be reported only.
                    fun platforms(key: String): List<String>? =
                        when (val value = document.valueAt("/figure/$it/$key")) {
                            null -> {
                                null
                            }

                            is JSONValue.Array -> {
                                value.values.indices.map { place -> text("/figure/$it/$key/$place") }.onEach { platform ->
                                    if (platform !in PLATFORMS) {
                                        throw RunnerFailure(
                                            "the figure `$name` names the platform `$platform`, which is neither ios nor android",
                                        )
                                    }
                                }
                            }

                            else -> {
                                throw RunnerFailure("the figure `$name`'s `$key` is not a list of platforms")
                            }
                        }
                    val measuring = platforms("platforms") ?: PLATFORMS
                    val unheld = platforms("unheld_on").orEmpty()
                    val flagged = flag("/figure/$it/flagged")
                    unheld.firstOrNull { platform -> platform !in measuring }?.let { platform ->
                        throw RunnerFailure("the figure `$name` is unheld on `$platform`, which does not measure it")
                    }
                    if (unheld.isNotEmpty() && !flagged) {
                        throw RunnerFailure("the figure `$name` is unheld on a platform and never flagged")
                    }
                    // A figure no platform holds is one the rule never acts on, which `flagged = false` says once.
                    if (unheld.isNotEmpty() && unheld.containsAll(measuring)) {
                        throw RunnerFailure("the figure `$name` is unheld everywhere it is measured, which `flagged = false` says")
                    }
                    BaselineFigure(
                        name,
                        number("/figure/$it/decimals").toInt(),
                        flagged,
                        measuring,
                        unheld,
                        condition(document, "/figure/$it", name, flagged),
                    )
                },
            )
        }
    }
}

/**
 * The most memory this process has held since it started, in bytes: the kernel's high-water mark of its resident
 * set, `VmHWM`, the one peak the platform keeps of an app's memory (`baseline.toml` says why).
 */
internal fun peakResidentBytes(): Double = procBytes("/proc/self/status", "VmHWM").toDouble()

/** The median of [values]: the middle one, or the mean of the two in the middle. */
internal fun median(values: List<Double>): Double {
    val sorted = values.sorted()
    val middle = sorted.size / 2
    return if (sorted.size % 2 == 1) sorted[middle] else (sorted[middle - 1] + sorted[middle]) / 2
}

/**
 * The frequency cap of the phone's fastest cores, in MHz: the kernel's `scaling_max_freq` of the cores whose own top,
 * `cpuinfo_max_freq`, is the highest. The phone caps its own cores by charge, battery and temperature, its fastest
 * ones furthest, and a run's figures are that cap's (`baseline.toml` says how much); the highest cap of any core would
 * read a slower cluster's once the fastest is held below it.
 */
private fun cpuCapMhz(): Double {
    fun khz(
        core: File,
        name: String,
    ) = File(core, "cpufreq/$name")
        .takeIf { it.canRead() }
        ?.readText()
        ?.trim()
        ?.toLongOrNull()
    val cores =
        File("/sys/devices/system/cpu")
            .listFiles()
            .orEmpty()
            .filter { it.name.matches(Regex("cpu\\d+")) }
            .mapNotNull { core -> khz(core, "cpuinfo_max_freq")?.let { top -> khz(core, "scaling_max_freq")?.let { cap -> top to cap } } }
    val fastest = cores.maxOfOrNull { it.first } ?: error("no core says its top frequency and its cap")
    return cores.filter { it.first == fastest }.maxOf { it.second } / 1000.0
}

/**
 * A surface of [side] pixels square whose consumer takes each frame as it arrives, as an app's view shows each: an
 * image reader that closes every image on a thread of its own when it comes, counting it. A reader nobody reads holds
 * the frames it was given, and once its buffers are full every present waits a second for one and draws nothing, so
 * a run's later previews would time the wait.
 */
private class ShownSurface(
    side: Int,
) : AutoCloseable {
    private val thread = HandlerThread("baseline-view").apply { start() }
    private val shown = Semaphore(0)
    private val reader =
        reader(side).apply {
            setOnImageAvailableListener({ frames -> frames.acquireNextImage()?.use { shown.release() } }, Handler(thread.looper))
        }

    /** The surface an app's view would hand over. */
    val surface: Surface get() = reader.surface

    /** Whether [frames] more frames are shown within [WAIT_FOR_FRAME_MILLIS], each counted once. */
    fun shows(frames: Int): Boolean = shown.tryAcquire(frames, WAIT_FOR_FRAME_MILLIS, TimeUnit.MILLISECONDS)

    /** The frames shown and not yet counted by [shows]. */
    val uncounted: Int get() = shown.availablePermits()

    override fun close() {
        reader.close()
        thread.quitSafely()
    }
}

/** The longest a run keeps the phone awake: far longer than its measurement takes, so a hung run still lets the phone sleep. */
private const val AWAKE_FOR_MILLIS = 10 * 60 * 1000L

/** The most the two clocks may part by over an awake run: each counts in whole milliseconds. */
private const val SLEPT_MILLIS = 5L

/**
 * What [measure] answers, made with the phone kept awake, as it is while its user watches an app export. A phone that
 * nothing keeps awake, off its cable with the screen off, suspends in the middle of an export: the monotonic clock the
 * figures read stops while it sleeps, but every waking costs the export time of its own, so the run is slower by what
 * the phone's sleep decides. The time the phone slept, which [SystemClock.elapsedRealtime] counts and
 * [SystemClock.uptimeMillis] does not, must be none.
 */
private suspend fun <T> keptAwake(
    context: Context,
    measure: suspend () -> T,
): T {
    val lock = context.getSystemService(PowerManager::class.java).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "craftbridge:baseline")
    lock.acquire(AWAKE_FOR_MILLIS)
    try {
        val asleep = { SystemClock.elapsedRealtime() - SystemClock.uptimeMillis() }
        val before = asleep()
        val measured = measure()
        val slept = asleep() - before
        require(slept <= SLEPT_MILLIS) { "the phone slept $slept ms during the run, which no figure of it counts" }
        return measured
    } finally {
        if (lock.isHeld) lock.release()
    }
}

/** The battery's temperature in degrees Celsius, as the platform's battery broadcast gives it in tenths. */
private fun batteryCelsius(context: Context): Double {
    val battery = context.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED)) ?: error("no battery broadcast")
    val tenths = battery.getIntExtra(BatteryManager.EXTRA_TEMPERATURE, Int.MIN_VALUE)
    require(tenths != Int.MIN_VALUE) { "the battery broadcast has no temperature" }
    return tenths / 10.0
}

/** The card, opened and filled as Maya fills it and drawn at [seconds] into [target]: the path of a preview. */
private suspend fun EffectsEngine.previewedCard(
    template: File,
    photo: File,
    target: Surface,
    seconds: Double,
): TemplateInstance {
    val card = templates.open(template.path)
    card.set("Name", "Sam".jsonValue)
    card.set("Photo", photo)
    preview(target, comp = card.comp, time = seconds.seconds)
    return card
}

/** The length of the movie at [file], in seconds, as the platform reads it. */
private fun movieSeconds(file: File): Double {
    val reader = MediaMetadataRetriever()
    try {
        reader.setDataSource(file.path)
        return (reader.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLong() ?: error("${file.path} has no duration")) /
            1000.0
    } finally {
        reader.release()
    }
}

/**
 * The running example's card template, made the way Leo makes one: a composition of the example's size, a text layer
 * and a red picture as imported footage, a Name control on the text and a Photo control on the picture, saved as
 * `card.ectemplate` in [folder]. The steps name the layers and items, not their ids.
 */
private suspend fun EffectsEngine.makeCardTemplate(
    folder: File,
    example: BaselineExample,
): File {
    val photo = File(folder, "photo.png")
    val template = File(folder, "card.ectemplate")
    photo.writeBytes(solidPng(0xFF, 0x00, 0x00))
    run(
        listOf(
            RunStep(
                "comp.new",
                mapOf(
                    "name" to "Card".jsonValue,
                    "width" to example.width.toDouble().jsonValue,
                    "height" to example.height.toDouble().jsonValue,
                    "duration" to example.seconds.toDouble().jsonValue,
                    "frameRate" to example.frameRate.toDouble().jsonValue,
                ),
            ),
            RunStep("layer.newSolid", mapOf("comp" to "Card".jsonValue, "name" to "Bg".jsonValue, "color" to "#223344".jsonValue)),
            RunStep("layer.newText", mapOf("comp" to "Card".jsonValue, "text" to "Hello".jsonValue, "name" to "Name".jsonValue)),
            RunStep("file.import", mapOf("paths" to JSONValue.Array(listOf(photo.path.jsonValue)))),
            RunStep("layer.addItem", mapOf("comp" to "Card".jsonValue, "item" to "photo.png".jsonValue)),
            RunStep(
                "essential.addProperty",
                mapOf(
                    "comp" to "Card".jsonValue,
                    "layer" to "Name".jsonValue,
                    "path" to "text/sourceText".jsonValue,
                    "name" to "Name".jsonValue,
                ),
            ),
            RunStep("essential.addMedia", mapOf("layer" to "photo.png".jsonValue, "name" to "Photo".jsonValue)),
            RunStep("essential.exportTemplate", mapOf("comp" to "Card".jsonValue, "path" to template.path.jsonValue)),
        ),
    )
    require(template.isFile) { "the template file was not written: ${template.path}" }
    return template
}

/** An 8 by 8 picture of one colour as a PNG: a photo the app's picker could hand over. */
private fun solidPng(
    red: Int,
    green: Int,
    blue: Int,
): ByteArray = pngOfStraight(ByteArray(8 * 8 * 4) { index -> listOf(red, green, blue, 0xFF)[index % 4].toByte() }, 8, 8)

internal val baselineChecks: List<Case> =
    listOf(
        Case(
            "baseline: the file defines the running example and its figures, and the line says them by name in the file's order",
        ) { context ->
            val baseline = Baseline.of(context)
            agreed(baseline.line, "craftbridge-baseline", "the word the line starts with")
            agreed(
                baseline.figures.map { listOf(it.name, it.decimals, it.flagged, it.platforms, it.unheldOn, it.condition) },
                listOf(
                    listOf("first_preview_ms", 1, true, listOf("ios", "android"), listOf("android"), null),
                    listOf("gpu_start_ms", 1, false, listOf("ios", "android"), emptyList<String>(), null),
                    listOf("first_path_ms", 1, true, listOf("ios", "android"), listOf("android"), null),
                    listOf("preview_path_ms", 1, true, listOf("ios", "android"), listOf("android"), null),
                    listOf("full_export_ms", 1, true, listOf("ios", "android"), emptyList<String>(), null),
                    listOf("peak_memory_bytes", 0, true, listOf("ios", "android"), emptyList<String>(), null),
                    listOf("export_threads", 0, false, listOf("ios", "android"), emptyList<String>(), null),
                    listOf("thermal_state", 0, false, listOf("ios"), emptyList<String>(), BaselineCondition.AtMost(1.0)),
                    listOf("cpu_cap_mhz", 0, false, listOf("android"), emptyList<String>(), BaselineCondition.Record),
                    listOf("battery_c", 1, false, listOf("android"), emptyList<String>(), BaselineCondition.AtMost(33.0)),
                ),
                "the figures",
            )
            val example = baseline.example
            agreed(
                listOf(
                    example.width,
                    example.height,
                    example.seconds,
                    example.frameRate,
                    example.frames,
                    example.previewAtSeconds,
                    example.previewRepeats,
                ),
                listOf(1080, 1080, 6, 30, 180, 2.0, 9),
                "the running example",
            )
            agreed(example.format, "h264", "the running example's format")
            agreed(median(listOf(5.0, 1.0, 3.0)), 3.0, "the median of three")
            agreed(median(listOf(4.0, 1.0, 3.0, 2.0)), 2.5, "the median of four")
            agreed(
                baseline.line(
                    mapOf(
                        "first_preview_ms" to 1234.56,
                        "gpu_start_ms" to 1000.0,
                        "first_path_ms" to 234.56,
                        "preview_path_ms" to 23.4,
                        "full_export_ms" to 6789.0,
                        "peak_memory_bytes" to 123_456_789.4,
                        "export_threads" to 8.0,
                        "cpu_cap_mhz" to 2880.0,
                        "battery_c" to 32.9,
                    ),
                    listOf(30.0, 1.25, 20.004, 22.0, 21.5, 40.0, 25.0, 24.0, 23.0),
                ),
                "craftbridge-baseline first_preview_ms=1234.6 gpu_start_ms=1000.0 first_path_ms=234.6 preview_path_ms=23.4 " +
                    "full_export_ms=6789.0 " +
                    "peak_memory_bytes=123456789 export_threads=8 cpu_cap_mhz=2880 battery_c=32.9 " +
                    "preview_repeats_ms=30.00,1.25,20.00,22.00,21.50,40.00,25.00,24.00,23.00",
                "the line",
            )
            val dated = runCatching { Baseline.of("recorded = 2026-11-19\n") }.exceptionOrNull()
            require(
                dated?.message?.startsWith("the baseline holds a TOML value JSON has none for") == true,
            ) { "a TOML date in the baseline was answered ${dated?.message ?: "with no refusal"}" }
            val broken = runCatching { Baseline.of("[running_example\n") }.exceptionOrNull()
            require(broken?.message?.startsWith("the baseline is not TOML") == true) {
                "a baseline that is not TOML was answered ${broken?.message ?: "with no refusal"}"
            }
            val file = assetText(context, BASELINE_ASSET)
            val unheld = "unheld_on = [\"android\"]"
            val iosOnly = "platforms = [\"ios\"]"
            val atMost = "$iosOnly\ncondition = \"at_most\""
            val recorded = "platforms = [\"android\"]\ncondition = \"record\""
            listOf(
                file.replaceFirst(unheld, "unheld_on = [\"ios\", \"android\"]") to
                    "the figure `first_preview_ms` is unheld everywhere it is measured, which `flagged = false` says",
                file.replaceFirst(unheld, "unheld_on = [\"andriod\"]") to
                    "the figure `first_preview_ms` names the platform `andriod`, which is neither ios nor android",
                file.replaceFirst(unheld, "unheld_on = \"android\"") to
                    "the figure `first_preview_ms`'s `unheld_on` is not a list of platforms",
                file.replaceFirst(unheld, "unheld_in = [\"android\"]") to
                    "the figure `first_preview_ms` holds the key `unheld_in`, which no figure has",
                file.replaceFirst(iosOnly, "$iosOnly\nunheld_on = [\"android\"]") to
                    "the figure `thermal_state` is unheld on `android`, which does not measure it",
                file.replaceFirst(iosOnly, "$iosOnly\nunheld_on = [\"ios\"]") to
                    "the figure `thermal_state` is unheld on a platform and never flagged",
                file.replaceFirst(iosOnly, "platforms = \"ios\"") to "the figure `thermal_state`'s `platforms` is not a list of platforms",
                file.replaceFirst("$atMost\ncondition_bound = 1\n", "$atMost\n") to
                    "the figure `thermal_state` is `at_most` and names no number `condition_bound`",
                file.replaceFirst("$atMost\ncondition_bound = 1", "$iosOnly\ncondition_bound = 1") to
                    "the figure `thermal_state` holds a `condition_bound` and no `condition`",
                file.replaceFirst(atMost, "$iosOnly\ncondition = \"atmost\"") to
                    "the figure `thermal_state`'s `condition` is `atmost`, which is neither record nor at_most",
                file.replaceFirst(atMost, "$iosOnly\ncondition = 1") to
                    "the figure `thermal_state`'s `condition` is not text, which is neither record nor at_most",
                file.replaceFirst(recorded, "$recorded\ncondition_bound = 2880") to
                    "the figure `cpu_cap_mhz` is `record` and holds a `condition_bound`, which it has no use for",
                file.replaceFirst("flagged = false\nplatforms = [\"ios\"]", "flagged = true\nplatforms = [\"ios\"]") to
                    "the figure `thermal_state` is a condition and flagged, which only a cost is",
            ).forEach { (spelt, refusal) ->
                agreed(runCatching { Baseline.of(spelt) }.exceptionOrNull()?.message, refusal, "a figure spelt otherwise")
            }
            val lacking = runCatching { baseline.line(mapOf("first_preview_ms" to 1.0), emptyList()) }.exceptionOrNull()
            agreed(
                lacking?.message,
                "the run measured [first_preview_ms], and the baseline names [first_preview_ms, gpu_start_ms, first_path_ms, " +
                    "preview_path_ms, full_export_ms, peak_memory_bytes, export_threads, cpu_cap_mhz, battery_c]",
                "a run that lacks a figure",
            )
            val figuresOfRun = baseline.measured.associate { it.name to 1.0 }
            // One repeat fewer and one more than the example makes: the nearest a wrong count comes, which a looser rule
            // would let through.
            listOf(
                8 to "the run made 8 warm previews, and the running example makes 9",
                10 to "the run made 10 warm previews, and the running example makes 9",
            ).forEach { (made, refused) ->
                agreed(
                    runCatching { baseline.line(figuresOfRun, List(made) { 20.0 }) }.exceptionOrNull()?.message,
                    refused,
                    "a run that made a warm preview fewer or more",
                )
            }
            // The file's own word and decimals spelt otherwise, so a line that wrote either itself rather than reading
            // them would show it.
            val respelt =
                Baseline.of(
                    file
                        .replaceFirst("repeats_word = \"preview_repeats_ms\"", "repeats_word = \"warm_ms\"")
                        .replaceFirst("repeats_decimals = 2", "repeats_decimals = 3"),
                )
            agreed(
                respelt.line(figuresOfRun, List(9) { 20.0004 }).substringAfterLast(" "),
                "warm_ms=" + List(9) { "20.000" }.joinToString(","),
                "the repeats under the word and with the decimals the file gives",
            )
        },
        Case("baseline: the running example's first preview, full export, peak memory and conditions are said on one line") { context ->
            val baseline = Baseline.of(context)
            val example = baseline.example
            val folder = File(Folders.appDefault(context).cache, "baseline")
            folder.deleteRecursively()
            folder.mkdirs()
            try {
                // The app bundles its template: an engine of its own makes it, and is gone before the clock starts.
                val maker = startOn(context)
                val template =
                    try {
                        maker.makeCardTemplate(folder, example)
                    } finally {
                        maker.shutDown()
                    }
                val sam = File(folder, "sam.png").apply { writeBytes(solidPng(0x00, 0xFF, 0x00)) }
                // A surface of the card's own size in pixels, shown as an app's view shows it.
                val target = ShownSurface(example.width)
                val movie = File(folder, "card.mp4")
                val cap = cpuCapMhz()
                val battery = batteryCelsius(context)
                val line =
                    keptAwake(context) {
                        val clock = TimeSource.Monotonic
                        val started = clock.markNow()
                        val engine = EffectsEngine.start(context, gpu = GpuChoice.PREFERRED)
                        try {
                            // The first call waits behind the engine thread's own start of the GPU, which ends when it answers.
                            engine.request("info", JSONValue.Object(emptyMap()))
                            val gpuStart = started.elapsedNow().inWholeMicroseconds / 1000.0
                            val card = engine.previewedCard(template, sam, target.surface, example.previewAtSeconds)
                            val firstPreview = started.elapsedNow().inWholeMicroseconds / 1000.0

                            var counted = 0uL
                            val exporting = clock.markNow()
                            engine.export(
                                request =
                                    mapOf(
                                        "comp" to card.comp,
                                        "output" to movie.path.jsonValue,
                                        "format" to example.format.jsonValue,
                                    ),
                                onProgress = { _, total, _ -> counted = total },
                            )
                            val fullExport = exporting.elapsedNow().inWholeMicroseconds / 1000.0
                            // Read before the checks below, which no app of the example's makes, and while the engine that
                            // exported still holds its pool.
                            val peak = peakResidentBytes()
                            val threads = exportThreads().toDouble()
                            val paths =
                                List(example.previewRepeats) {
                                    val began = clock.markNow()
                                    engine.previewedCard(template, sam, target.surface, example.previewAtSeconds)
                                    began.elapsedNow().inWholeMicroseconds / 1000.0
                                }
                            engine.endPreview()
                            val previews = 1 + example.previewRepeats
                            require(
                                target.shows(previews),
                            ) { "the surface showed ${target.uncounted} of the $previews previews drawn into it" }
                            require(
                                counted == example.frames.toULong(),
                            ) { "the export counted $counted frames, not the running example's ${example.frames}" }
                            require(
                                abs(movieSeconds(movie) - example.seconds) < 0.1,
                            ) { "the movie does not last the running example's ${example.seconds} s" }
                            val figures =
                                mapOf(
                                    "first_preview_ms" to firstPreview,
                                    "gpu_start_ms" to gpuStart,
                                    "first_path_ms" to firstPreview - gpuStart,
                                    "preview_path_ms" to median(paths),
                                    "full_export_ms" to fullExport,
                                    "peak_memory_bytes" to peak,
                                    "export_threads" to threads,
                                    "cpu_cap_mhz" to cap,
                                    "battery_c" to battery,
                                )
                            val costs = baseline.measured.filter { it.flagged }.map { it.name } + "export_threads"
                            require(costs.all { figures.getValue(it) > 0 }) { "a figure is no measurement: $figures" }
                            baseline.line(figures, paths)
                        } finally {
                            engine.shutDown()
                            target.close()
                        }
                    }
                say(line)
            } finally {
                folder.deleteRecursively()
            }
        },
    )
