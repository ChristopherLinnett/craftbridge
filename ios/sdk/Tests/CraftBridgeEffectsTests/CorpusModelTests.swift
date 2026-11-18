// What the runner reads of the corpus, and how: the model refuses what it does not read, as the desktop
// runner's does, so a check the corpus gains is never silently left unrun; TOML's values become the JSON
// the engine takes; and a pointer and a fraction mean what they do on the desktop.

import CraftBridge
import Foundation
import Testing

@Suite struct CorpusModelTests {
    /// A corpus of one scenario and one step, with `rest` after the step's own keys.
    private func corpus(_ rest: String = "", step: String = "") -> String {
        """
        [[scenario]]
        id = "X"
        title = "x"
        [[scenario.step]]
        \(step)
        [[scenario.step.call]]
        kind = "request"
        method = "info"
        \(rest)

        """
    }

    /// Why a corpus is refused, or nil when it parses.
    private func refusal(of text: String) -> String? {
        do {
            _ = try Corpus(toml: text)
            return nil
        } catch {
            return String(describing: error)
        }
    }

    @Test func aCorpusOfOneScenarioReadsWithItsDefaults() throws {
        let scenario = try #require(try Corpus(toml: corpus()).scenarios.first)
        #expect((scenario.id, scenario.title, scenario.sides) == ("X", "x", .both))
        #expect(scenario.steps.count == 1 && scenario.expects.isEmpty && scenario.folders.isEmpty)
        let step = try #require(scenario.steps.first)
        #expect(step.host == nil && step.stdin.isEmpty && step.calls.count == 1)
        guard case .request("info", let params, let files, let stop)? = step.calls.first else {
            Issue.record("the step's call is not the request of `info`: \(step.calls)")
            return
        }
        #expect(params == [:] && files.isEmpty && stop == nil)
    }

    @Test func aKeyOrAKindTheRunnerDoesNotReadIsRefusedNotIgnored() {
        let unread = "the runner does not read the key"
        let refused: [(text: String, why: String)] = [
            (corpus().replacingOccurrences(of: "title =", with: "titel ="), "\(unread) `titel`"),
            (corpus(step: "surplus = 1"), "\(unread) `surplus`"),
            (corpus("surplus = 1"), "\(unread) `surplus`"),
            (corpus("progress = { cancel_after = 0.5 }"), "\(unread) `cancel_after`"),
            (corpus("[scenario.step.host]\nsurplus = 1\n"), "\(unread) `surplus`"),
            (
                corpus().replacingOccurrences(of: "kind = \"request\"\nmethod", with: "kind = \"mcp\"\nmethod"),
                "\(unread) `method`"
            ),
            (
                corpus().replacingOccurrences(of: "method = \"info\"", with: "method = \"info\"\nformat = \"png\""),
                "\(unread) `format`"
            ),
            (
                corpus() + "[[scenario.expect]]\nstep = 0\npointer = \"\"\nlength = 1\nsurplus = 1\n",
                "\(unread) `surplus`"
            ),
            (corpus() + "[[scenario.folder]]\npath = \".\"\nholds = []\nsurplus = 1\n", "\(unread) `surplus`"),
            (corpus() + "[[scenario.fixture]]\nfrom = \"a\"\nto = \"b\"\nsurplus = 1\n", "\(unread) `surplus`"),
            (corpus() + "[[scenario.same]]\nkind = \"timings\"\nstep = 0\nomit = []\n", "\(unread) `omit`"),
            (
                corpus()
                    + "[[scenario.compare]]\nkind = \"identical\"\nactual = \"a\"\nreference = \"b\"\nexact = true\n",
                "\(unread) `exact`"
            ),
            (corpus().replacingOccurrences(of: "kind = \"request\"", with: "kind = \"launch\""), "launch"),
            (corpus() + "[[scenario.compare]]\nkind = \"similar\"\nactual = \"a\"\nreference = \"b\"\n", "similar"),
            ("surplus = 1\n" + corpus(), "\(unread) `surplus`"),
        ]
        for (text, why) in refused {
            let said = refusal(of: text)
            #expect(
                said?.contains(why) == true, "expected a refusal naming \(why), got \(said ?? "none") for:\n\(text)")
        }
    }

    @Test func theKeysEachKindOfCheckHasAreRead() throws {
        let text =
            corpus(
                "progress = { cancel_at = 0.5 }\nfiles = { script = \"a.jsonl\" }",
                step: "args = [\"x\"]\nfails = true\nstdin = [\"l\"]")
                + """
                [[scenario.expect]]
                step = 0
                output = 1
                pointer = "/a"
                equals = 90
                [[scenario.same]]
                kind = "output"
                step = 0
                omit = ["/a"]
                entries = [{ pointer = "", key = "id", prefix = "x", omit = ["/b"] }]
                within = [{ pointer = "/t", omit = ["/c"] }]
                [[scenario.compare]]
                kind = "pixels"
                actual = "a.png"
                reference = "b.png"
                tolerance = 1
                allowed_fraction = 0.003
                cap = 4
                [[scenario.compare]]
                kind = "frames"
                actual = "a.mp4"
                reference = "b.png"
                reference_frame = 0
                exact = true
                [[scenario.folder]]
                path = "."
                holds = ["a"]
                [[scenario.fixture]]
                from = "a"
                to = "b"
                names_container = true
                """
        let scenario = try #require(try Corpus(toml: text).scenarios.first)
        let step = try #require(scenario.steps.first)
        #expect(step.stdin == ["l"])
        guard case .request(_, _, let files, let stop)? = step.calls.first else {
            Issue.record("no request: \(step.calls)")
            return
        }
        #expect(files == ["script": "a.jsonl"] && stop?.cancelAt == 0.5)
        let expect = try #require(scenario.expects.first)
        #expect((expect.step, expect.output, expect.pointer, expect.equals) == (0, 1, "/a", 90))
        #expect(scenario.sames.count == 1 && scenario.compares.map(\.actual) == ["a.png", "a.mp4"])
        #expect(scenario.compares.map(\.pixels) == [Tolerance(levels: 1, allowedFraction: 0.003, cap: 4), nil])
        let plain = try #require(
            try Corpus(
                toml: corpus()
                    + "[[scenario.compare]]\nkind = \"pixels\"\nactual = \"a\"\nreference = \"a\"\ntolerance = 2\n"
            ).scenarios.first?.compares.first)
        #expect(
            plain.pixels == Tolerance(levels: 2, allowedFraction: 0, cap: 2), "the cap is the tolerance unless named")
        #expect(scenario.folders.first?.holds == ["a"] && scenario.fixtures.first?.namesContainer == true)
    }

    @Test func tomlValuesBecomeTheJSONTheEngineTakes() throws {
        let scenario = try #require(
            try Corpus(
                toml: corpus(
                    """
                    params = { a = 1, b = 0.5, c = true, d = "t", e = [1, "x"], f = { g = 2 } }
                    """
                )
            ).scenarios.first
        )
        guard case .request(_, let params, _, _)? = scenario.steps.first?.calls.first else {
            Issue.record("no request")
            return
        }
        #expect(params == ["a": 1, "b": 0.5, "c": true, "d": "t", "e": [1, "x"], "f": ["g": 2]])
        let date = refusal(of: corpus("params = { when = 1979-05-27 }"))
        #expect(date?.contains("the corpus holds a TOML value JSON has none for") == true, "\(date ?? "")")
    }

    @Test func aPointerNamesAValueByRFC6901AndAMissingOneIsNil() {
        let document: JSONValue = ["a": ["b": [1, ["c": "x"]]], "k/l": 1, "m~n": 2, "": 3]
        #expect(document.value(at: "") == document)
        #expect(document.value(at: "/a/b/1/c") == "x")
        #expect(document.value(at: "/a/b/0") == 1)
        #expect(document.value(at: "/k~1l") == 1 && document.value(at: "/m~0n") == 2)
        #expect(document.value(at: "/") == 3, "an empty key is a key")
        for missing in ["/a/b/2", "/a/b/-1", "/a/b/x", "/z", "/a/b/0/c", "a"] {
            #expect(document.value(at: missing) == nil, "\(missing)")
        }
    }

    @Test func aCallIsStoppedAtTheFractionOfItsFramesAndNeverBeforeItHasSaidHowManyItHas() throws {
        func stop(_ fraction: String) throws -> Stop {
            let scenario = try #require(try Corpus(toml: corpus("progress = \(fraction)")).scenarios.first)
            guard case .request(_, _, _, let stop?)? = scenario.steps.first?.calls.first else {
                throw RunnerFailure(description: "no stop in \(fraction)")
            }
            return stop
        }
        let none = try stop("{}")
        #expect(!none.stops(done: 100, total: 100))
        let half = try stop("{ cancel_at = 0.5 }")
        #expect(
            !half.stops(done: 49, total: 100) && half.stops(done: 50, total: 100) && half.stops(done: 51, total: 100))
        #expect(!half.stops(done: 0, total: 0), "a call that has not said how many frames it has is not stopped")
        let whole = try stop("{ cancel_at = 1 }")
        #expect(whole.stops(done: 10, total: 10) && !whole.stops(done: 9, total: 10))
    }
}
