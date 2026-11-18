// Why a call over the native boundary failed: one exception per row of the plan's mapping table (§14).

package io.github.christopherlinnett.craftbridge.core

import io.github.christopherlinnett.craftbridge.ffi.core.craftErrorMessage
import io.github.christopherlinnett.craftbridge.ffi.core.CraftException as BoundaryException

/**
 * The failure a call over the native boundary answers with: one exception for each row of the plan's
 * mapping table, and nothing else ever reaches an app. Each says what an app can act on — a parameter
 * to fix, a comp to make first, a call to stop asking, a disk to free — and [CancelledException] is
 * the answer of a call the engine was told to stop.
 *
 * [message] is the sentence the core gives the failure, read from the core itself so that no second
 * copy of its words can fall out of step with it, which is what an app shows its user; the fields
 * beside it are for the app that acts on the failure rather than only reporting it. The core's calls
 * make them with the core's words. An app makes one in its own tests, to see how its code handles that
 * failure without an engine, and gives each constructor the words it wants there as `message`.
 */
public sealed class CraftException protected constructor(
    message: String,
) : Exception(message)

/** Unknown parameter, bad JSON, or parameters a check refuses. */
public class InvalidParamsException(
    /** Whichever check refused, in its own words. */
    public val detail: String,
    message: String,
) : CraftException(message)

/** A command answered that it failed, or one step of a `run` did. */
public class CommandFailedException(
    /** The command that failed, as it is named to the engine. */
    public val command: String,
    /** The command's own sentence about why it failed. */
    public val detail: String,
    /** Which step of a `run` failed, or none when the call was not a `run`. */
    public val step: UInt?,
    message: String,
) : CraftException(message)

/** No comp, layer or property the call named. */
public class NotFoundException(
    /** What was looked for, in the engine's words. */
    public val detail: String,
    message: String,
) : CraftException(message)

/** A capability this platform or build lacks, such as an export format it does not carry. */
public class UnsupportedException(
    /** What is not possible here, in the engine's words, which names what it does answer. */
    public val detail: String,
    message: String,
) : CraftException(message)

/**
 * The call stopped because its cancel token was set: the export is wound down and its half-written
 * files are gone. A call whose own coroutine was cancelled ends as that cancel instead, a
 * `CancellationException` with this as its cause, since a cancelled coroutine that throws anything else
 * fails the scope it runs in.
 */
public class CancelledException(
    message: String,
) : CraftException(message)

/** A file access the engine may not make or the disk will not do: missing, unreadable, unwritable, outside the container, full. */
public class IoException(
    /** The folder or file, and what went wrong with it, in the operating system's words. */
    public val detail: String,
    message: String,
) : CraftException(message)

/** A graphics device or surface that would not be made. */
public class GpuException(
    /** The device or surface that failed, in the driver's or the core's words. */
    public val detail: String,
    message: String,
) : CraftException(message)

/** A failure no row of the table fits: a caught panic, after which the engine keeps the document, or a call it can no longer answer. */
public class InternalErrorException(
    /** What the engine found, in its own words. */
    public val detail: String,
    message: String,
) : CraftException(message)

/**
 * The core's exception for what the boundary answered with. Each variant is one row of the table, and
 * [words] is the sentence the core gives that failure: the boundary carries the parts of a failure and
 * the core alone knows how they read, so the words are asked of it ([mapped]) and not rebuilt here.
 */
internal fun BoundaryException.asCoreException(words: String): CraftException =
    when (this) {
        is BoundaryException.InvalidParams -> InvalidParamsException(detail, words)
        is BoundaryException.CommandFailed -> CommandFailedException(command, detail, step, words)
        is BoundaryException.NotFound -> NotFoundException(detail, words)
        is BoundaryException.Unsupported -> UnsupportedException(detail, words)
        is BoundaryException.Cancelled -> CancelledException(words)
        is BoundaryException.Io -> IoException(detail, words)
        is BoundaryException.Gpu -> GpuException(detail, words)
        is BoundaryException.Internal -> InternalErrorException(detail, words)
    }

/**
 * Make one call over the boundary and fail with the core's exception for whatever the boundary
 * answered: an app never sees a boundary type, and never a crash, because every failure the engine can
 * answer is a row of the table (AGENTS.md §3).
 */
internal inline fun <R> mapped(call: () -> R): R =
    try {
        call()
    } catch (refused: BoundaryException) {
        throw refused.asCoreException(craftErrorMessage(refused))
    }
