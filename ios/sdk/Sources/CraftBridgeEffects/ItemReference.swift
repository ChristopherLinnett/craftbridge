// How a command names a project item, a layer, an effect or a property.

import CraftBridge

/// A reference to an item by number or by name, as the engine's `id|name` keys take it.
public enum ItemReference: Sendable, Hashable, JSONRepresentable {
    /// An id or an index: the engine reads either as a number.
    case number(Int)
    /// A name, or a uid, which the engine reads as a string.
    case name(String)

    /// The reference as the engine takes it.
    public var jsonValue: JSONValue {
        switch self {
        case .number(let value): .number(Double(value))
        case .name(let value): .string(value)
        }
    }
}

extension ItemReference: ExpressibleByIntegerLiteral {
    /// An integer literal is a number reference.
    public init(integerLiteral value: Int) { self = .number(value) }
}

extension ItemReference: ExpressibleByStringLiteral {
    /// A string literal is a name reference.
    public init(stringLiteral value: String) { self = .name(value) }
}
