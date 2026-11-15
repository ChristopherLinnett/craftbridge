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
}
