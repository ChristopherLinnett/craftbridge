// An export's progress, handed to a lambda.

package io.github.christopherlinnett.craftbridge.core

import io.github.christopherlinnett.craftbridge.ffi.core.ProgressListener as BoundaryProgressListener

/**
 * What a call that reports progress hands its reports to: `done` of `total`, in the export's own units,
 * and the seconds left when the engine can say, `null` while it cannot.
 *
 * The engine reports on its own thread as the export runs, once every progress beat, so a handler
 * returns quickly and hands anything slow — drawing a progress bar — to the thread that draws it.
 */
public typealias ProgressHandler = (done: ULong, total: ULong, secondsLeft: Double?) -> Unit

/** The boundary's listener, which hands every report the engine makes to a lambda. */
internal class ProgressOf(
    private val handler: ProgressHandler,
) : BoundaryProgressListener {
    override fun progress(
        done: ULong,
        total: ULong,
        secondsLeft: Double?,
    ) {
        handler(done, total, secondsLeft)
    }
}
