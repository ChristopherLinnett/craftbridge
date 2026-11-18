// Steps that reach an engine one at a time, in the order they were asked for.

/// Runs steps on the main actor one at a time, each once every step handed in before it has ended.
///
/// An actor alone lets another call in at every `await`, so two steps that each ask the engine something
/// and then hand it something can interleave there, and what they hand it can reach it in either order:
/// a step that remembers what it handed over would then remember what the engine did not take last.
@MainActor
final class Turns {
    private var last: Task<Void, Never>?

    /// Turns with no step handed in yet.
    nonisolated init() {}

    /// Runs `step` once every step handed in before it has ended, and answers what it answered.
    func run<Answer: Sendable>(_ step: @escaping @MainActor () async -> Answer) async -> Answer {
        let before = last
        let mine = Task {
            await before?.value
            return await step()
        }
        last = Task { _ = await mine.value }
        return await mine.value
    }

    /// Waits until every step handed in so far has ended.
    func settled() async {
        await last?.value
    }
}
