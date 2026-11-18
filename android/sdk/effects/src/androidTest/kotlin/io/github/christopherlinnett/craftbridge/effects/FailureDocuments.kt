// The document a failing call prints (plan §28): `{error, kind}`, as the desktop's `--json` prints `{error}`, with
// the core's own words and the name of the row of the plan's table the failure is. Each exception is imported
// under the name of its row, which is the kind it prints as.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.CraftException
import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.core.CancelledException as Cancelled
import io.github.christopherlinnett.craftbridge.core.CommandFailedException as CommandFailed
import io.github.christopherlinnett.craftbridge.core.GpuException as Gpu
import io.github.christopherlinnett.craftbridge.core.InternalErrorException as Internal
import io.github.christopherlinnett.craftbridge.core.InvalidParamsException as InvalidParams
import io.github.christopherlinnett.craftbridge.core.IoException as Io
import io.github.christopherlinnett.craftbridge.core.NotFoundException as NotFound
import io.github.christopherlinnett.craftbridge.core.UnsupportedException as Unsupported

/**
 * The kind of a failure, as the plug-in's callers tell failures apart. Exhaustive, so a new kind is named
 * here before the checks build.
 */
internal fun kindOf(failure: CraftException): String =
    when (failure) {
        is InvalidParams -> "InvalidParams"
        is CommandFailed -> "CommandFailed"
        is NotFound -> "NotFound"
        is Unsupported -> "Unsupported"
        is Cancelled -> "Cancelled"
        is Io -> "Io"
        is Gpu -> "Gpu"
        is Internal -> "Internal"
    }

/** A call that failed, as the document its step prints: a failure is an answer a scenario can check, as the desktop's is. */
internal fun documentOf(failure: CraftException): JSONValue =
    JSONValue.Object(mapOf("error" to JSONValue.Text(failure.message.orEmpty()), "kind" to JSONValue.Text(kindOf(failure))))

/** The check that the document is the core's words and the kind's name, for every row of the table. */
internal val failureDocumentCheck =
    Case("a failing call prints {error, kind} as the desktop does, in the core's words and the kind's name") {
        val failures =
            listOf(
                InvalidParams("a", "words of a") to "InvalidParams",
                CommandFailed("c", "b", null, "words of b") to "CommandFailed",
                NotFound("c", "words of c") to "NotFound",
                Unsupported("d", "words of d") to "Unsupported",
                Cancelled("words of e") to "Cancelled",
                Io("f", "words of f") to "Io",
                Gpu("g", "words of g") to "Gpu",
                Internal("h", "words of h") to "Internal",
            )
        for ((failure, kind) in failures) {
            agreed(
                documentOf(failure),
                obj("error" to str(failure.message.orEmpty()), "kind" to str(kind)),
                "the document of a $kind failure",
            )
        }
    }
