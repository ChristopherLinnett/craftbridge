// The sentences of the core's failures, read through the real boundary, where the library that holds them loads.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.CommandFailedException
import io.github.christopherlinnett.craftbridge.core.Engine
import io.github.christopherlinnett.craftbridge.core.InvalidParamsException
import io.github.christopherlinnett.craftbridge.core.IoException
import io.github.christopherlinnett.craftbridge.core.NotFoundException

/**
 * What an engine's own refusals say, as the exceptions an app catches carry it. The sentence of each is
 * the core's, read from the core, and only the native library can give it, so it is pinned here and not
 * on the host: a failed command keeps its name and its step around its reason, and every other row's
 * message is the reason as the engine or the system worded it. The `Gpu` and `Internal` rows have no
 * call that answers them on a device that works, so they are pinned by the boundary's own tests of the
 * core's words (`craftbridge-core-ffi`'s `error.rs`) and by the host's tests of the mapping.
 */
internal val wordsChecks: List<Case> =
    listOf(
        Case("a call the engine refuses answers in the core's own words, the failed command's step among them") { context ->
            onEngine(context) { engine ->
                val badStep = """{"steps":[{"command":"comp.new","params":{}},{"command":"file.open","params":{}}]}"""
                val failedInARun = refusalOf(engine, "run", badStep)
                require(failedInARun is CommandFailedException) { "a failed step of a run was answered $failedInARun" }
                require(failedInARun.step == 1u) { "the failed step was ${failedInARun.step}, not 1" }
                requireWords(failedInARun, "file.open: missing `path` (step 1)")

                val failedAlone = refusalOf(engine, "open", "{}")
                require(failedAlone is CommandFailedException) { "a failed command was answered $failedAlone" }
                require(failedAlone.step == null) { "a command that was not part of a run named step ${failedAlone.step}" }
                requireWords(failedAlone, "file.open: missing `path`")

                val unknown = refusalOf(engine, "run", """{"steps":[{"command":"nosuch.cmd","params":{}}]}""")
                require(unknown is InvalidParamsException) { "an unknown command was answered $unknown" }
                requireWords(unknown, "nosuch.cmd: unknown command `nosuch.cmd` (step 0)")
                require(unknown.detail == unknown.message) { "the detail ${unknown.detail} is not the message ${unknown.message}" }

                val missing = refusalOf(engine, "props", """{"comp":"zzz","layer":"#1"}""")
                require(missing is NotFoundException) { "a comp that is not there was answered $missing" }
                requireWords(missing, "no composition \"zzz\"")
                require(missing.detail == missing.message) { "the detail ${missing.detail} is not the message ${missing.message}" }

                val outside = refusalOf(engine, "open", """{"path":"/nonexistent/x.ecp"}""")
                require(outside is IoException) { "a file outside the automation roots was answered $outside" }
                require(outside.message.orEmpty().startsWith("cannot read `/nonexistent/x.ecp`: it is outside the automation roots")) {
                    "the refusal of a file outside the roots reads ${outside.message}"
                }
                require(outside.detail == outside.message) { "the detail ${outside.detail} is not the message ${outside.message}" }
            }
        },
    )

/** What the engine answers a call with, which has to be a refusal. */
private suspend fun refusalOf(
    engine: Engine,
    method: String,
    params: String,
): Throwable =
    runCatching { engine.request(method, params) }.exceptionOrNull()
        ?: error("`$method` $params was answered, and the check needs a refusal")

private fun requireWords(
    refusal: Throwable,
    words: String,
) {
    require(refusal.message == words) { "the refusal reads ${refusal.message}, not $words" }
}
