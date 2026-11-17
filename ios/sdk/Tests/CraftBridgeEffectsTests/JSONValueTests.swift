import CraftBridge
import Foundation
import Testing

@Suite struct JSONValueTests {
    @Test func everyKindOfJSONSurvivesAWriteAndARead() throws {
        let value: JSONValue = ["a": [1, 2.5, "x", true, nil], "b": ["c": false]]
        let data = try JSONEncoder().encode(value)
        #expect(try JSONDecoder().decode(JSONValue.self, from: data) == value)
    }

    @Test func aDocumentIsReadAsWhatItSays() throws {
        let data = Data(#"{"n":null,"t":true,"d":1.5,"s":"s","a":[],"o":{}}"#.utf8)
        let value = try JSONDecoder().decode(JSONValue.self, from: data)
        #expect(value == ["n": nil, "t": true, "d": 1.5, "s": "s", "a": [], "o": [:]])
    }

    @Test func theTypedValuesAreTheJSONTheEngineTakes() {
        #expect(Vec2(1, 2).jsonValue == [1, 2])
        #expect(Vec3(1, 2, 3).jsonValue == [1, 2, 3])
        #expect(CraftColor.rgb(1, 0.5, 0).jsonValue == [1, 0.5, 0])
        #expect(CraftColor.hex("#ff8800").jsonValue == "#ff8800")
        #expect(true.jsonValue == true)
        #expect(2.5.jsonValue == 2.5)
        #expect("a".jsonValue == "a")
    }

    @Test func aValueCrossesAsTheJSONTextItHolds() throws {
        let value: JSONValue = ["output": "/Library/Caches/card.mp4", "end": 4, "queue": false, "rendered": [nil]]
        #expect(try JSONValue(jsonText: try value.jsonText()) == value)
        #expect(try JSONValue(jsonText: #"{"rendered":[{"seconds":1.5}]}"#) == ["rendered": [["seconds": 1.5]]])
        // An answer is any JSON, a bare value too.
        #expect(try JSONValue(jsonText: "null") == .null)
        #expect(try JSONValue(jsonText: #""done""#) == "done")
    }

    @Test func parametersJSONHasNoNumberForAreRefusedAsTheCallsParameters() {
        let refusal = CraftError.InvalidParams(
            detail: "the parameters hold NaN or an infinity, which JSON has no number for")
        for value: JSONValue in [.number(.nan), ["end": [.number(.infinity)]], [.number(-.infinity)]] {
            #expect(throws: refusal) { try value.jsonText() }
        }
    }

    @Test func anAnswerThatIsNotJSONIsTheEnginesOwnFailure() {
        let failure = CraftError.Internal(detail: "the engine answered with text that is not JSON")
        for text in ["", "{", "{\"rendered\": }", "NaN"] {
            #expect(throws: failure) { try JSONValue(jsonText: text) }
        }
    }
}
