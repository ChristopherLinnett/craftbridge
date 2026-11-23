// A running export: the answer to await, and the way to stop it.

import CraftBridgeFFI

/// A call that runs for a while and reports its progress, such as an export, under way from the moment
/// the job is made: await ``result`` for its answer, or ``resultJSON`` for the engine's own text of it, and
/// ``cancel()`` it, or cancel the task awaiting either, to stop it.
///
/// UniFFI's futures cannot be cancelled, so the job stops through the cancel token it hands the engine,
/// which the engine reads at the export's next progress report and answers with ``CraftError/Cancelled``.
/// The job's call is made from a task of its own, so a call made on the same engine just after the job
/// may reach the engine before it; one that must follow the job is made once its answer has come.
/// A job nobody holds still runs to its end.
public final class ExportJob: Sendable {
    private let token: CancelToken
    private let call: Task<String, any Error>

    /// Starts `method` of the engine's method table on `engine`, with `params`, handing its progress to
    /// `progress`. Parameters JSON cannot carry are refused here, with ``CraftError/InvalidParams(detail:)``,
    /// and nothing is started.
    package convenience init(
        engine: Engine,
        method: String,
        params: JSONValue,
        progress: @escaping ProgressHandler = { _, _, _ in }
    ) throws {
        self.init(engine: engine, method: method, paramsJSON: try params.jsonText(), progress: progress)
    }

    /// Starts `method` of the engine's method table on `engine`, with the parameters `paramsJSON` exactly as
    /// given, handing its progress to `progress`. The engine's text is kept as it was written, and
    /// ``resultJSON`` and ``result`` both answer from it. Text that is not JSON is refused by the engine,
    /// which fails the job.
    package init(
        engine: Engine,
        method: String,
        paramsJSON: String,
        progress: @escaping ProgressHandler = { _, _, _ in }
    ) {
        let token = CancelToken()
        let listener = ClosureProgressListener(progress)
        self.token = token
        call = Task {
            try await engine.requestWithProgress(
                method: method,
                params: paramsJSON,
                listener: listener,
                cancel: token
            )
        }
    }

    /// Asks the engine to stop the job at its next progress report, after which ``result`` and
    /// ``resultJSON`` throw ``CraftError/Cancelled``. Safe from any thread and at any time: a job stays
    /// cancelled, and one that answered before the engine read the request keeps its answer.
    public func cancel() {
        token.cancel()
    }

    /// Whether the job was asked to stop, by ``cancel()`` or by cancelling a task that awaits ``result`` or
    /// ``resultJSON``.
    public var isCancelled: Bool { token.isCancelled() }

    /// The job's answer as the engine's own text, exactly as it wrote it, or the ``CraftError`` it failed
    /// with: the reading for a binding to another language, which keeps what a ``JSONValue`` cannot (a
    /// number written `2.0`, the order of an object's keys, and a whole number above 2^53 digit for digit).
    /// An app prefers ``result``. Cancelling the task that awaits it cancels the job, as ``cancel()`` does,
    /// so a job awaited in a task the app cancels stops with it. It can be awaited any number of times, from
    /// any task.
    public var resultJSON: String {
        get async throws {
            try await withTaskCancellationHandler {
                try await call.value
            } onCancel: {
                cancel()
            }
        }
    }

    /// The job's answer, as the engine gives it, or the ``CraftError`` it failed with: ``resultJSON`` read as
    /// a ``JSONValue``, so text the engine wrote that is not JSON fails here, each time, with
    /// ``CraftError/Internal(detail:)``. Cancelling the task that awaits it cancels the job, as ``cancel()``
    /// does. It can be awaited any number of times, from any task.
    public var result: JSONValue {
        get async throws {
            try JSONValue(jsonText: try await resultJSON)
        }
    }
}
