// The entry point of the EffectCraft API: the engine's start, the fixed-shape calls of the plan's
// table, and behind them the generated command namespaces in `generated/` hanging off it.

package io.github.christopherlinnett.craftbridge.effects

import android.content.Context
import android.graphics.Bitmap
import android.hardware.HardwareBuffer
import android.view.Surface
import io.github.christopherlinnett.craftbridge.core.CraftException
import io.github.christopherlinnett.craftbridge.core.Engine
import io.github.christopherlinnett.craftbridge.core.Folders
import io.github.christopherlinnett.craftbridge.core.GpuException
import io.github.christopherlinnett.craftbridge.core.InvalidParamsException
import io.github.christopherlinnett.craftbridge.core.JSONRepresentable
import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.core.NamedArguments
import io.github.christopherlinnett.craftbridge.core.NotFoundException
import io.github.christopherlinnett.craftbridge.core.ProgressHandler
import io.github.christopherlinnett.craftbridge.core.UnsupportedException
import io.github.christopherlinnett.craftbridge.core.jsonValue
import io.github.christopherlinnett.craftbridge.ffi.core.FrameBytes
import io.github.christopherlinnett.craftbridge.ffi.core.GpuChoice
import io.github.christopherlinnett.craftbridge.ffi.core.PixelFormat
import io.github.christopherlinnett.craftbridge.ffi.effects.startEffectsEngine
import kotlin.time.Duration
import kotlin.time.DurationUnit

/** An EffectCraft engine: the fixed-shape calls of the plan's table, and the typed command
 * namespaces over its one generic request call, which the `generated/` facade hangs off this. */
public class EffectsEngine internal constructor(
    /** The universal core engine behind this one: what its exports, its frame and MCP calls, and its
     * end all go through. The typed [preview], [frameBitmap] and [frameHardwareBuffer] go through it, and an app
     * reaches the engine's own report of the preview surface (`hasPreviewSurface`, `holdsPreview`), the
     * surface's raw attach ([Engine.attachPreview]) and the GPU's life (`suspendGpu`, `resumeGpu`) on it. */
    public val core: Engine,
    /** How one request of the engine's method table is sent and its reply read back: JSON text
     * through [core] for a real engine, a recording stand-in for a host test of the shapes sent. */
    private val transport: suspend (method: String, params: JSONValue) -> JSONValue = engineTransport(core),
) {
    public companion object {
        /**
         * Start an EffectCraft engine: its own thread, its folders at [folders] — the app's own by
         * default, the ones Android gives it to keep and to share — and its drawing on [gpu] where it
         * can, the CPU everywhere else. The engine keeps its document in memory and nothing on disk
         * until a call writes one. It starts through the core's own start, which hands the device's codec
         * list over first: an engine started past it would decode every stream in software.
         *
         * @throws CraftException when the engine refuses to come up.
         */
        public fun start(
            context: Context,
            folders: Folders = Folders.appDefault(context),
            gpu: GpuChoice = GpuChoice.PREFERRED,
        ): EffectsEngine = EffectsEngine(Engine.start(folders) { boundaryFolders -> startEffectsEngine(boundaryFolders, gpu) })
    }

    /** Sends one request of the engine's method table, such as `"exec"`, and returns its reply. Internal:
     * the public calls are the typed ones built on it. */
    internal suspend fun request(
        method: String,
        params: JSONValue,
    ): JSONValue = transport(method, params)

    /** Sends one request that reports progress, through the core, where the engine's cancel token lives. */
    internal suspend fun requestWithProgress(
        method: String,
        params: JSONValue,
        onProgress: ProgressHandler,
    ): JSONValue = jsonValueOf(core.requestWithProgress(method, jsonText(params), onProgress))

    /** Asks the method [name] for its answer, taking its parameters from these fields. The one road
     * every fixed-shape call of this table but `exec`, `run` and `export` walks. */
    private suspend fun ask(
        name: String,
        vararg field: Pair<String, JSONValue?>,
    ): JSONValue = request(name, fields(*field))

    /** The engine's `exec` envelope: the command's id and its parameters, written the one way. */
    private fun execEnvelope(
        command: String,
        params: JSONValue,
    ): JSONValue = JSONValue.Object(mapOf("command" to JSONValue.Text(command), "params" to params))

    /** Runs the command `id`. `extra` holds keys the type rules left untyped, none when it is `null`, and
     * `params` the keys the caller typed, a `null` value for one left out; a typed key replaces an extra
     * one, so a typed value is not lost to a stray key. The generated facade passes its keys as pairs,
     * so its own code calls no library function a command or key of the same name could take. */
    internal suspend fun command(
        id: String,
        extra: Map<String, JSONValue>?,
        vararg params: Pair<String, JSONValue?>,
    ): JSONValue {
        val keys = extra.orEmpty().toMutableMap()
        for ((key, value) in params) {
            if (value != null) {
                keys[key] = value
            }
        }
        return request("exec", execEnvelope(id, JSONValue.Object(keys)))
    }

    /**
     * Run one command, id and parameters as the engine takes them, and answer its reply. The generated
     * namespaces run their commands through this; an app reaches for it for a command no namespace names yet.
     *
     * @throws CraftException with whatever the engine answers the command with.
     */
    public suspend fun exec(
        command: String,
        params: Map<String, JSONValue> = emptyMap(),
    ): JSONValue = request("exec", execEnvelope(command, JSONValue.Object(params)))

    /**
     * Run a list of commands in order, and answer the list of their replies. Every step is read before
     * the first runs, so a malformed step anywhere runs none of them; a step that fails stops the run
     * there, and the steps before it keep what they did.
     *
     * @throws CraftException naming the step, with what the engine answered the failing step with.
     */
    public suspend fun run(steps: List<RunStep>): JSONValue =
        request(
            "run",
            JSONValue.Object(
                mapOf("steps" to JSONValue.Array(steps.map { step -> execEnvelope(step.command, JSONValue.Object(step.params)) })),
            ),
        )

    /**
     * Answer the property rows of [layer] — its id, its `#n` place counted from the top, or its name —
     * in [comp], the active comp when it is left out, as the engine's reply: what each property is, and
     * its value at [time] when one is given. [flat] asks rows without their tree, [depth] how deep the
     * tree goes.
     *
     * @throws CraftException when there are no such rows to answer: the engine's refusal comes back.
     */
    public suspend fun props(
        vararg namedOnly: NamedArguments,
        layer: JSONValue,
        comp: JSONValue? = null,
        flat: Boolean? = null,
        time: Double? = null,
        depth: ULong? = null,
    ): JSONValue =
        ask(
            "props",
            "comp" to comp,
            "layer" to layer,
            "flat" to flat?.jsonValue,
            "time" to time?.jsonValue,
            "depth" to depth?.toNumber(),
        )

    /**
     * Answer one property: [path] as the rows name it, of [layer] in [comp], read at [time] when the caller names one.
     *
     * @throws CraftException when no such property exists to read, in the engine's words.
     */
    public suspend fun get(
        vararg namedOnly: NamedArguments,
        layer: JSONValue,
        path: String,
        comp: JSONValue? = null,
        time: Double? = null,
    ): JSONValue =
        ask(
            "get",
            "comp" to comp,
            "layer" to layer,
            "path" to path.jsonValue,
            "time" to time?.jsonValue,
        )

    /**
     * Change one property: [path] as the rows name it, of [layer] in [comp], to [value], to the
     * [expression] that drives it, or both, at [time] for a keyframe write.
     *
     * @throws CraftException when the write cannot be made, as the engine refuses it.
     */
    public suspend fun set(
        vararg namedOnly: NamedArguments,
        layer: JSONValue,
        path: String,
        comp: JSONValue? = null,
        value: JSONRepresentable? = null,
        expression: String? = null,
        time: Double? = null,
    ): JSONValue =
        ask(
            "set",
            "comp" to comp,
            "layer" to layer,
            "path" to path.jsonValue,
            "value" to value?.jsonValue,
            "expression" to expression?.jsonValue,
            "time" to time?.jsonValue,
        )

    /**
     * Render one frame: of [comp] — the active comp when it is left out — at [time] in seconds or at
     * [frame], a frame number the comp's frame rate turns into seconds, and at the active comp's current
     * time with neither; [maxSide] pixels across its longest side or [scale] of it, full size with
     * neither; and with [transparent], its own alpha kept rather than composited over the comp's
     * background colour. The bytes come back as [format] asks, before any conversion to a picture: a
     * `Bitmap` or a `HardwareBuffer` of a frame is [frameBitmap] and [frameHardwareBuffer].
     *
     * @throws CraftException when the engine cannot render it.
     */
    public suspend fun frame(
        vararg namedOnly: NamedArguments,
        comp: JSONValue? = null,
        time: Double? = null,
        frame: Long? = null,
        maxSide: Long? = null,
        scale: Double? = null,
        transparent: Boolean? = null,
        format: PixelFormat = PixelFormat.PNG,
    ): FrameBytes = core.frame(frameRequest(comp, time, frame, maxSide, scale, transparent), format)

    /**
     * Render one frame as a [Bitmap], `ARGB_8888` and premultiplied: the frame [EffectsEngine.frame] asks for,
     * with the same arguments, holding the pixels the core's own premultiplied conversion gives (plan §17).
     *
     * @throws CraftException with the engine's refusal of the render, or when the phone has no memory to hold the bitmap.
     */
    public suspend fun frameBitmap(
        vararg namedOnly: NamedArguments,
        comp: JSONValue? = null,
        time: Double? = null,
        frame: Long? = null,
        maxSide: Long? = null,
        scale: Double? = null,
        transparent: Boolean? = null,
    ): Bitmap = core.frameBitmap(frameRequest(comp, time, frame, maxSide, scale, transparent))

    /**
     * Render one frame as a [HardwareBuffer], `RGBA_8888` and premultiplied, which the GPU and other
     * processes read as it is: the frame [EffectsEngine.frame] asks for, with the same arguments. The
     * caller closes the buffer. It needs Android 12 (API 31) or newer.
     *
     * @throws UnsupportedException on an older Android, and [CraftException] when the render is refused.
     */
    public suspend fun frameHardwareBuffer(
        vararg namedOnly: NamedArguments,
        comp: JSONValue? = null,
        time: Double? = null,
        frame: Long? = null,
        maxSide: Long? = null,
        scale: Double? = null,
        transparent: Boolean? = null,
    ): HardwareBuffer = core.frameHardwareBuffer(frameRequest(comp, time, frame, maxSide, scale, transparent))

    /**
     * Preview [comp] at [time] into [surface], the app's own: the surface is attached when the engine is
     * not previewing into this one yet — on the first call, on a new surface, on a [Surface] object that
     * names another window since (a `SurfaceView` hands out one object for every surface it makes), on a
     * surface lost with a GPU given up for the background, on one the app attached itself asking to carry the
     * composition's alpha ([Engine.attachPreview]), which this call attaches again over black, and after
     * [endPreview] gave it up — and the frame is drawn into it on the GPU. An app that keeps a surface attached
     * with that ask draws into it with the overload that names no surface. An app that draws many frames into
     * one surface names the same surface every time, and it is attached once: naming the surface the engine
     * already previews into costs no attach and no second configuration. Naming a surface after resizing it is
     * how the resize reaches the frame: the draw answers [GpuException] as the surface is configured again, and
     * the draw after it draws. [comp] is the active comp when it is left out, [time] its current time.
     *
     * @throws GpuException when there is no device to draw on or the surface is lost or changed, and
     * [CraftException] for whatever else the core's `attachPreview` or `present` refuse with.
     */
    public suspend fun preview(
        surface: Surface,
        vararg namedOnly: NamedArguments,
        comp: JSONValue? = null,
        time: Duration? = null,
    ) {
        if (!core.holdsPreview(surface)) core.attachPreview(surface)
        preview(comp = comp, time = time)
    }

    /**
     * Draw [comp] at [time] into the surface attached last: [comp] is the active comp when it is left out,
     * [time] its current time. The engine lets that surface go at [endPreview], and with the GPU at
     * [core]'s `suspendGpu`, so after either, as with none attached, it answers `GpuException` until a
     * surface is attached again, as the overload that takes one attaches it.
     *
     * @throws GpuException when the surface or the device fails, [CraftException] otherwise.
     */
    public suspend fun preview(
        vararg namedOnly: NamedArguments,
        comp: JSONValue? = null,
        time: Duration? = null,
    ) {
        core.present(previewRequest(comp, time))
    }

    /**
     * Fit a preview of [comp] at [time] into a box of [width] by [height] pixels, through the engine's
     * `previewRequest`: the `frame` request that draws it with its own alpha, as large as fits the box and
     * never above the comp's own pixels, and the comp's pixel size, which sets the size of the texture or
     * buffer the frame is drawn into. Leave [comp] out for the active comp and [time] out for its current
     * time. The same comp, time and box give the same answer on iOS.
     *
     * @throws InvalidParamsException naming [width] or [height] when it is below 1, and [NotFoundException]
     * when the comp is not there, as [frame] refuses it.
     */
    public suspend fun previewRequest(
        vararg namedOnly: NamedArguments,
        width: Long,
        height: Long,
        comp: JSONValue? = null,
        time: Duration? = null,
    ): PreviewRequest =
        previewRequestOf(
            ask(
                "previewRequest",
                "comp" to comp,
                "time" to time?.toDouble(DurationUnit.SECONDS)?.jsonValue,
                "width" to width.toNumber(),
                "height" to height.toNumber(),
            ),
        )

    /**
     * Ends the preview: the engine gives its preview surface up, and forgets it, so a later [preview] of
     * any surface attaches afresh. Ending a preview nobody started is not an error.
     *
     * @throws CraftException for whatever the core's `detachPreview` answers when it gives the surface up.
     */
    public suspend fun endPreview() {
        core.detachPreview()
    }

    /**
     * Export, watching it: [request] is the engine's render request as the plan's table gives it — the
     * comp, the format, the files — and [onProgress] hears every report the engine makes as it goes.
     * Cancelling the coroutine this runs in stops the export the engine's own way, its half-written files
     * gone, and the call ends as a cancelled coroutine ends.
     *
     * @throws CraftException when the engine refuses the request or answers a part of the render badly.
     */
    public suspend fun export(
        request: Map<String, JSONValue>,
        onProgress: ProgressHandler,
    ): JSONValue = requestWithProgress("export", JSONValue.Object(request), onProgress)

    /**
     * Answer the engine's own timings for [comp]: how long its frame at [time] takes at [scale], over
     * [runs] repeats; with [play], the cost of that many consecutive frames without the layer cache and
     * with it; and with [gpu], the same comp against the attached accelerator.
     *
     * @throws CraftException when the engine cannot time the comp; it says why.
     */
    public suspend fun timings(
        vararg namedOnly: NamedArguments,
        comp: JSONValue? = null,
        time: Double? = null,
        scale: Double? = null,
        runs: ULong? = null,
        play: ULong? = null,
        gpu: Boolean? = null,
    ): JSONValue =
        ask(
            "timings",
            "comp" to comp,
            "time" to time?.jsonValue,
            "scale" to scale?.jsonValue,
            "runs" to runs?.toNumber(),
            "play" to play?.toNumber(),
            "gpu" to gpu?.jsonValue,
        )

    /**
     * Replay a recorded command script, [script], and answer the engine's report of it. A step that fails
     * is part of that report, `ok: false` naming the first failure, not an exception; [keepGoing] runs
     * the steps after it too.
     *
     * @throws CraftException when the engine refuses the script before running any of it, and says why.
     */
    public suspend fun replay(
        script: String,
        keepGoing: Boolean? = null,
    ): JSONValue = ask("replay", "script" to script.jsonValue, "keepGoing" to keepGoing?.jsonValue)

    /**
     * Pass one line of the app's MCP transport through and answer with the line back, or `null`
     * when the line asks for none.
     *
     * @throws CraftException when the engine refuses the line outright.
     */
    public suspend fun mcp(line: String): String? = core.mcp(line)

    /**
     * End the engine: its thread finishes and every later call says there is no engine left to answer.
     *
     * @throws CraftException when the engine will not come to its end.
     */
    public suspend fun shutDown() {
        core.shutDown()
    }
}

/** The requests of a real engine go as JSON text through the core, and come back read as JSONValue. */
private fun engineTransport(core: Engine): suspend (String, JSONValue) -> JSONValue =
    { method, params -> jsonValueOf(core.request(method, jsonText(params))) }

/**
 * The one frame request the core's `frame`, `frameBitmap`, `frameHardwareBuffer` and `present` read, as
 * JSON text: a preview draws what a frame renders, so its request is these fields, the unnamed ones left out.
 */
private fun frameRequest(
    comp: JSONValue?,
    time: Double?,
    frame: Long?,
    maxSide: Long?,
    scale: Double?,
    transparent: Boolean?,
): String =
    jsonText(
        fields(
            "comp" to comp,
            "time" to time?.jsonValue,
            "frame" to frame?.toNumber(),
            "maxSide" to maxSide?.toNumber(),
            "scale" to scale?.jsonValue,
            "transparent" to transparent?.jsonValue,
        ),
    )

/**
 * The request a preview hands to the core's `present`: the `frame` fields of a picture — [comp] and the
 * preview [time] — with the rest left unnamed. This one place turns a `Duration` into the seconds the
 * engine speaks, so a preview's request is a frame's, byte for byte, whatever duration the app named.
 */
private fun previewRequest(
    comp: JSONValue?,
    time: Duration?,
): String = frameRequest(comp, time?.toDouble(DurationUnit.SECONDS), null, null, null, null)

/** The JSON object of these request fields, dropping the ones the caller left out. */
internal fun fields(vararg entries: Pair<String, JSONValue?>): JSONValue.Object =
    JSONValue.Object(entries.mapNotNull { (key, value) -> value?.let { key to it } }.toMap())

/** A whole number, carried in JSON's one number type. */
private fun Long.toNumber(): JSONValue = JSONValue.Number(toDouble())

private fun ULong.toNumber(): JSONValue = JSONValue.Number(toDouble())
