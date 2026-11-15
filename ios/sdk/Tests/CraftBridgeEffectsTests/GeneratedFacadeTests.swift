import CraftBridge
import CraftBridgeEffects
import Foundation
import Testing

/// What the engine was asked, kept for the test to read.
private actor Requests {
    private(set) var all: [(method: String, params: JSONValue)] = []

    func record(_ method: String, _ params: JSONValue) {
        all.append((method, params))
    }
}

private func engine(recording requests: Requests) -> EffectsEngine {
    EffectsEngine { method, params in
        await requests.record(method, params)
        return "done"
    }
}

@Suite struct GeneratedFacadeTests {
    @Test func aTypedCallSendsTheCommandWithItsKeysAsJSON() async throws {
        let requests = Requests()
        let reply = try await engine(recording: requests).layer.newText(
            text: "Hello",
            position: Vec2(10, 20),
            extra: ["unknownKey": true]
        )
        #expect(reply == "done")
        let sent = await requests.all
        #expect(sent.count == 1)
        #expect(sent.first?.method == "exec")
        let expected: JSONValue = [
            "command": "layer.newText",
            "params": ["text": "Hello", "position": [10, 20], "unknownKey": true],
        ]
        #expect(sent.first?.params == expected)
    }

    @Test func aKeyLeftOutIsNotSent() async throws {
        let requests = Requests()
        _ = try await engine(recording: requests).layer.newText()
        let expected: JSONValue = ["command": "layer.newText", "params": [:]]
        #expect(await requests.all.first?.params == expected)
    }

    @Test func aTypedKeyIsNotReplacedByAnExtraOne() async throws {
        let requests = Requests()
        _ = try await engine(recording: requests).layer.newText(text: "typed", extra: ["text": "stray", "other": 1])
        let expected: JSONValue = ["command": "layer.newText", "params": ["text": "typed", "other": 1]]
        #expect(await requests.all.first?.params == expected)
    }

    @Test func aNestedNamespaceIsReachedThroughItsParent() async throws {
        let requests = Requests()
        _ = try await engine(recording: requests).layer.mask.motionBlur(
            layer: .number(3),
            mask: "Mask 1",
            mode: .sameAsLayer
        )
        let expected: JSONValue = [
            "command": "layer.mask.motionBlur",
            "params": ["layer": 3, "mask": "Mask 1", "mode": "sameAsLayer"],
        ]
        #expect(await requests.all.first?.params == expected)
    }

    @Test func aReservedWordIsAKeyAllTheSame() async throws {
        let requests = Requests()
        _ = try await engine(recording: requests).footage.overlayEdit(in: 2)
        let sent = await requests.all.first?.params
        #expect(sent == ["command": "footage.overlayEdit", "params": ["in": 2]])
    }

    @Test func aCommandAndTheNamespaceBelowItKeepTheirIdsNames() async throws {
        let requests = Requests()
        let engine = engine(recording: requests)
        _ = try await engine.edit.history(steps: 2)
        _ = try await engine.edit.history.list()
        let sent = await requests.all.map(\.params)
        let expected: [JSONValue] = [
            ["command": "edit.history", "params": ["steps": 2]],
            ["command": "edit.history.list", "params": [:]],
        ]
        #expect(sent == expected)
    }

    @Test func aNamespaceNamedFromADigitTakesItsParentsName() async throws {
        let requests = Requests()
        _ = try await engine(recording: requests).view.view3d.front()
        #expect(await requests.all.first?.params == ["command": "view.3d.front", "params": [:]])
    }

    @Test func anErrorFromTheEngineIsThrown() async {
        struct Refused: Error {}
        let refusing = EffectsEngine { _, _ in throw Refused() }
        await #expect(throws: Refused.self) { try await refusing.layer.newText() }
    }

    @Test func aGeneratedEnumKeyAllowsNoExhaustiveSwitch() {
        // An emitted enum key is a `RawRepresentable` struct with a static member per word, not an enum,
        // so a word the engine adds later breaks no build that compiles today. Two things here hold that:
        // the compiler makes the `switch` carry a `default` clause, and any word makes a value without an
        // optional, which an enum's `init?(rawValue:)` would give and this target would then not compile.
        func word(of mode: EffectsEngine.LayerMaskCommands.MotionBlurMode) -> String {
            switch mode {
            case .sameAsLayer: "sameAsLayer"
            case .on: "on"
            case .off: "off"
            default: mode.rawValue
            }
        }
        #expect(word(of: .on) == "on")
        let written = EffectsEngine.LayerMaskCommands.MotionBlurMode(rawValue: "aWordWrittenLater")
        #expect(written.jsonValue == .string("aWordWrittenLater"))
    }
}
