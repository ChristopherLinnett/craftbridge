// What the engine's `previewRequest` answers: the frame a preview draws, fitted to its box, and the size of
// the composition behind it.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.JSONValue

/**
 * The answer of [EffectsEngine.previewRequest]: the `frame` request that draws the comp with its own alpha, as
 * large as fits the box and no larger than the comp's pixels, and the comp's pixel size, which sets the size of
 * the texture or buffer the frame is drawn into.
 */
public class PreviewRequest internal constructor(
    /** The `frame` request, as the fields [EffectsEngine.frame] takes. */
    public val request: JSONValue.Object,
    /**
     * [request] as the JSON text the engine wrote it in, which the core's `frame`, `frameBitmap`,
     * `frameHardwareBuffer` and `present` take: the one text Android and iOS answer for the same comp, time and
     * box, since the engine writes it and neither platform writes it again.
     */
    public val requestText: String,
    /** The comp's width in pixels, which sets the width of the texture or buffer a frame is drawn into. */
    public val contentWidth: Long,
    /** The comp's height in pixels, which sets the height of the texture or buffer a frame is drawn into. */
    public val contentHeight: Long,
)

/** The [PreviewRequest] the engine's `previewRequest` [answer] holds. */
internal fun previewRequestOf(answer: JSONValue): PreviewRequest {
    val fields = (answer as? JSONValue.Object)?.entries
    val request = fields?.get("request") as? JSONValue.Object
    val text = (fields?.get("requestText") as? JSONValue.Text)?.value
    val width = (fields?.get("contentWidth") as? JSONValue.Number)?.value?.toWholeNumber()
    val height = (fields?.get("contentHeight") as? JSONValue.Number)?.value?.toWholeNumber()
    if (request == null || text == null || width == null || height == null) {
        throw engineAnsweredBadly("the engine's `previewRequest` answered a document without a request and a pixel size")
    }
    return PreviewRequest(request, text, width, height)
}

/** A whole number of pixels, which JSON carries as its one number type, or none for any other figure. */
private fun Double.toWholeNumber(): Long? = if (rem(1.0) == 0.0 && this in 0.0..MOST_PIXELS) toLong() else null

/** The most pixels a side can be named by: past it a double no longer holds every whole number. */
private const val MOST_PIXELS = 9.0e15
