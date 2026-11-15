// The JSON the native boundary speaks, and the values every Craft app's calls are made of.

import Foundation

/// A JSON value: what a request carries to the engine and what its reply holds.
public enum JSONValue: Sendable, Hashable, Codable {
    /// JSON's `null`.
    case null
    /// A JSON boolean.
    case bool(Bool)
    /// A JSON number. JSON has one number type, so integers are carried as `Double`.
    case number(Double)
    /// A JSON string.
    case string(String)
    /// A JSON array.
    case array([JSONValue])
    /// A JSON object.
    case object([String: JSONValue])

    /// Reads the value JSON holds, whichever kind it is.
    public init(from decoder: any Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([JSONValue].self) {
            self = .array(value)
        } else {
            self = .object(try container.decode([String: JSONValue].self))
        }
    }

    /// Writes the value as JSON.
    public func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null: try container.encodeNil()
        case .bool(let value): try container.encode(value)
        case .number(let value): try container.encode(value)
        case .string(let value): try container.encode(value)
        case .array(let value): try container.encode(value)
        case .object(let value): try container.encode(value)
        }
    }
}

extension JSONValue: ExpressibleByNilLiteral {
    /// `nil` is JSON's `null`.
    public init(nilLiteral: ()) { self = .null }
}

extension JSONValue: ExpressibleByBooleanLiteral {
    /// A boolean literal is a JSON boolean.
    public init(booleanLiteral value: Bool) { self = .bool(value) }
}

extension JSONValue: ExpressibleByIntegerLiteral {
    /// An integer literal is a JSON number.
    public init(integerLiteral value: Int) { self = .number(Double(value)) }
}

extension JSONValue: ExpressibleByFloatLiteral {
    /// A floating-point literal is a JSON number.
    public init(floatLiteral value: Double) { self = .number(value) }
}

extension JSONValue: ExpressibleByStringLiteral {
    /// A string literal is a JSON string.
    public init(stringLiteral value: String) { self = .string(value) }
}

extension JSONValue: ExpressibleByArrayLiteral {
    /// An array literal is a JSON array.
    public init(arrayLiteral elements: JSONValue...) { self = .array(elements) }
}

extension JSONValue: ExpressibleByDictionaryLiteral {
    /// A dictionary literal is a JSON object; a key written twice keeps its last value.
    public init(dictionaryLiteral elements: (String, JSONValue)...) {
        self = .object(Dictionary(elements, uniquingKeysWith: { _, last in last }))
    }
}

/// A value that is passed to an engine as JSON.
public protocol JSONRepresentable: Sendable {
    /// The value as the engine takes it.
    var jsonValue: JSONValue { get }
}

extension JSONValue: JSONRepresentable {
    /// A JSON value is its own JSON.
    public var jsonValue: JSONValue { self }
}

extension Bool: JSONRepresentable {
    /// A JSON boolean.
    public var jsonValue: JSONValue { .bool(self) }
}

extension Double: JSONRepresentable {
    /// A JSON number.
    public var jsonValue: JSONValue { .number(self) }
}

extension String: JSONRepresentable {
    /// A JSON string.
    public var jsonValue: JSONValue { .string(self) }
}

/// A value the engine takes as a word, passed as that word.
public protocol JSONStringRepresentable: JSONRepresentable, RawRepresentable<String> {}

extension JSONStringRepresentable {
    /// The word, as a JSON string.
    public var jsonValue: JSONValue { .string(rawValue) }
}

/// A point or size in two dimensions, passed as `[x, y]`.
public struct Vec2: Sendable, Hashable, JSONRepresentable {
    /// The first component.
    public var x: Double
    /// The second component.
    public var y: Double

    /// A vector of the given components.
    public init(_ x: Double, _ y: Double) {
        self.x = x
        self.y = y
    }

    /// The components as a JSON array.
    public var jsonValue: JSONValue { [.number(x), .number(y)] }
}

/// A point or size in three dimensions, passed as `[x, y, z]`.
public struct Vec3: Sendable, Hashable, JSONRepresentable {
    /// The first component.
    public var x: Double
    /// The second component.
    public var y: Double
    /// The third component.
    public var z: Double

    /// A vector of the given components.
    public init(_ x: Double, _ y: Double, _ z: Double) {
        self.x = x
        self.y = y
        self.z = z
    }

    /// The components as a JSON array.
    public var jsonValue: JSONValue { [.number(x), .number(y), .number(z)] }
}

/// A colour, passed as `[r, g, b]` or as a hexadecimal string.
public enum CraftColor: Sendable, Hashable, JSONRepresentable {
    /// Red, green and blue, as the engine reads them.
    case rgb(Double, Double, Double)
    /// A hexadecimal colour such as `"#ff8800"`.
    case hex(String)

    /// The colour as the engine takes it.
    public var jsonValue: JSONValue {
        switch self {
        case .rgb(let red, let green, let blue): [.number(red), .number(green), .number(blue)]
        case .hex(let value): .string(value)
        }
    }
}
