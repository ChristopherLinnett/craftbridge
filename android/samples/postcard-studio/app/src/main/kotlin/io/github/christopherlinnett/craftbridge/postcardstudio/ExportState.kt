package io.github.christopherlinnett.craftbridge.postcardstudio

import java.io.File

/** Where the card's export is, as the screen and the notification both tell it. */
sealed interface ExportState {
    /** No export has begun, or the last one's outcome has been dealt with. */
    data object Idle : ExportState

    /** The export is running, [done] of [total] frames made. */
    data class Running(
        val done: ULong,
        val total: ULong,
    ) : ExportState {
        /** How far the export has come, from 0 to 1, or 0 until the engine has said how many frames there are. */
        val fraction: Float get() = if (total == 0uL) 0f else (done.toDouble() / total.toDouble()).toFloat()
    }

    /** The movie is made, at [movie], and is offered to share. */
    data class Finished(
        val movie: File,
    ) : ExportState

    /** The export was stopped, by the user or by the system's own time limit, and left no movie. */
    data object Stopped : ExportState

    /** The export failed for [reason], in the SDK's own words. */
    data class Failed(
        val reason: String,
    ) : ExportState
}
