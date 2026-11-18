// A call that reports progress, listened to and stopped as the scenario says. The engine cannot be told to
// hold an export where the listener stops it, as the desktop runner holds one, so the stop reaches the
// export when the engine's next beat brings it to the listener, and a quick export may have ended by then.
// Neither the answer nor the folder says which: the core answers `Cancelled` and deletes the files an export
// listed once it has been told to stop, whether or not the export ended while the stop was on its way. The
// render queue says it: the exporter that meets a stop answers that it was cancelled, which the queue
// records on the export's item as `UserStopped`, and one that had rendered every frame first records `Done`.
// So a stopped call is judged by its item, never by how far the beats say it had got, which a stop slow to
// reach the export would make it pass.

package io.github.christopherlinnett.craftbridge.effects

import android.util.Log
import io.github.christopherlinnett.craftbridge.core.CraftException
import io.github.christopherlinnett.craftbridge.core.JSONValue
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
import java.util.concurrent.atomic.AtomicReference

/** One report of the engine's progress. */
internal data class Report(
    val done: ULong,
    val total: ULong,
)

/** How a call that reports progress is listened to, and when it is stopped. */
internal class Listening(
    private val stop: Stop,
) {
    internal companion object {
        /** The status the render queue gives the item of an export that met its stop. */
        val stopped: JSONValue = JSONValue.Text("UserStopped")

        /** The status of each item of the render queue, as `renderQueue.list` answers it in [list]. */
        fun statusesOf(list: JSONValue): List<JSONValue> {
            val items =
                (list.valueAt("/items") as? JSONValue.Array)?.values
                    ?: throw RunnerFailure("the render queue lists no items: ${list.shown}")
            return items.map { it.valueAt("/status") ?: JSONValue.Null }
        }

        /** The status of each item of the engine's render queue, as `renderQueue.list` gives it. */
        suspend fun statuses(engine: EffectsEngine): List<JSONValue> = statusesOf(engine.exec("renderQueue.list"))
    }

    /**
     * The call's answer, with the listener asking the call to stop once the fraction of its frames the
     * scenario names is done. A call that stops at a fraction is answered only when its export met the
     * stop, which [judge] decides from the render queue.
     *
     * The stop is the cancel of the coroutine that makes the call, the SDK's own way to stop an export. It is
     * made with the lock held and started after, so the listener, which waits on the lock for its first
     * report, always finds the coroutine it is to cancel: a stop never comes before there is a call to stop.
     */
    suspend fun run(
        method: String,
        params: JSONValue,
        engine: EffectsEngine,
    ): JSONValue {
        val lock = Any()
        val stoppedAt = AtomicReference<Report?>(null)
        val answer = AtomicReference<Result<JSONValue>?>(null)
        coroutineScope {
            synchronized(lock) {
                lateinit var call: Job
                call =
                    launch(start = CoroutineStart.LAZY) {
                        answer.set(
                            answered {
                                engine.requestWithProgress(method, params) { done, total, _ ->
                                    synchronized(lock) {
                                        if (stoppedAt.get() == null && stop.stops(done, total)) {
                                            stoppedAt.set(Report(done, total))
                                            call.cancel()
                                        }
                                    }
                                }
                            },
                        )
                    }
                call.start()
            }
        }
        val heard = stoppedAt.get()
        val statuses = if (heard == null) emptyList() else statuses(engine)
        if (heard != null) {
            Log.i(LOG_TAG, "stopped at ${heard.done} of ${heard.total} frames: the render queue says ${JSONValue.Array(statuses).shown}")
        }
        judge(heard, statuses)?.let { throw RunnerFailure(it) }
        return (answer.get() ?: throw RunnerFailure("the call ended with no answer")).getOrThrow()
    }

    /**
     * Why a call the scenario stops at a fraction proves nothing about the stop, or null when it does, given
     * the report the listener stopped it at and the statuses of the render queue's items once it answered:
     * the call must have been stopped there, and an export of it must have met the stop.
     */
    fun judge(
        stoppedAt: Report?,
        statuses: List<JSONValue>,
    ): String? {
        val cancelAt = stop.cancelAt ?: return null
        val race = "the export won the race, which the SDK has no way to hold"
        if (stoppedAt == null) {
            return "the call answered before its export reached ${JSONValue.Number(
                cancelAt,
            ).shown} of its frames, so no stop reached it: $race"
        }
        if (stopped !in statuses) {
            return "the stop came at ${stoppedAt.done} of ${stoppedAt.total} frames, and the render queue says " +
                "${JSONValue.Array(
                    statuses,
                ).shown}, with no export ${stopped.shown}: every frame was rendered before the stop reached the " +
                "export, so $race"
        }
        return null
    }
}

/**
 * What [call] answers, or the failure it ends with: a call stopped through its coroutine ends as a
 * cancelled coroutine does, with the engine's own answer as the cause, which is the failure the scenario reads.
 */
internal suspend fun answered(call: suspend () -> JSONValue): Result<JSONValue> =
    try {
        Result.success(call())
    } catch (stopped: CancellationException) {
        Result.failure(stopped.cause as? CraftException ?: throw stopped)
    } catch (refused: CraftException) {
        Result.failure(refused)
    }
