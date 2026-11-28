// The typed call that makes a request of the corpus: the request's keys read into the arguments the typed
// call names them by. A typed call spells a request only when it has an argument for every key the request
// names, so a request with a key no argument takes has no typed call here, and neither has a method the
// typed calls do not cover (`info`, `open`, `saveAs`, `commands`), which is what the generic door is for.

import CraftBridge
import CraftBridgeEffects
import Foundation

/// A request no typed call spells.
private struct Unspelt: Error {}

/// `value`, which the typed call cannot be made without.
private func needed<Value>(_ value: Value?) throws -> Value {
    guard let value else { throw Unspelt() }
    return value
}

/// A request's keys, taken one at a time by the arguments of a typed call, each as the argument's own type.
/// A key whose value is of another type is one the typed call cannot spell.
private struct Keys {
    private var fields: [String: JSONValue]

    init(_ params: JSONValue) throws {
        guard case .object(let fields) = params else { throw Unspelt() }
        self.fields = fields
    }

    /// The value under `key`, whatever it is, taken out of the keys left.
    mutating func value(_ key: String) -> JSONValue? {
        fields.removeValue(forKey: key)
    }

    mutating func string(_ key: String) throws -> String? {
        guard let value = value(key) else { return nil }
        guard case .string(let text) = value else { throw Unspelt() }
        return text
    }

    mutating func number(_ key: String) throws -> Double? {
        guard let value = value(key) else { return nil }
        guard case .number(let number) = value else { throw Unspelt() }
        return number
    }

    mutating func whole(_ key: String) throws -> Int? {
        try number(key).map { try needed(Int(exactly: $0)) }
    }

    mutating func flag(_ key: String) throws -> Bool? {
        guard let value = value(key) else { return nil }
        guard case .bool(let flag) = value else { throw Unspelt() }
        return flag
    }

    mutating func object(_ key: String) throws -> [String: JSONValue]? {
        guard let value = value(key) else { return nil }
        guard case .object(let fields) = value else { throw Unspelt() }
        return fields
    }

    /// An item named by its number or by its name, as the engine's `id|name` keys take one.
    mutating func reference(_ key: String) throws -> ItemReference? {
        switch value(key) {
        case nil: return nil
        case .number(let number)?: return .number(try needed(Int(exactly: number)))
        case .string(let name)?: return .name(name)
        default: throw Unspelt()
        }
    }

    /// The keys no argument has taken, for a call that hands on the ones it does not name.
    mutating func rest() -> [String: JSONValue] {
        defer { fields = [:] }
        return fields
    }

    /// Refuses a request that names a key no argument took.
    func taken() throws {
        guard fields.isEmpty else { throw Unspelt() }
    }
}

/// What `spell` makes of a request, or nil when it finds the request one no typed call spells.
private func spelt<Call>(_ spell: () throws -> Call) -> Call? {
    do {
        return try spell()
    } catch {
        return nil
    }
}

/// The steps of a `run` request as the typed `run` takes them. The typed call names every step's
/// parameters, so a step the corpus writes without any is made with none.
private func steps(of keys: inout Keys) throws -> [(command: String, params: JSONValue)] {
    guard case .array(let written)? = keys.value("steps") else { throw Unspelt() }
    return try written.map { step in
        var keys = try Keys(step)
        let command = try needed(keys.string("command"))
        let params = keys.value("params") ?? [:]
        try keys.taken()
        return (command, params)
    }
}

/// The typed call that makes the request `method` with `params` and answers at once, or nil when no typed
/// call spells it.
func typedRequest(_ method: String, _ params: JSONValue) -> (@Sendable (EffectsEngine) async throws -> JSONValue)? {
    spelt {
        var keys = try Keys(params)
        let call: @Sendable (EffectsEngine) async throws -> JSONValue
        switch method {
        case "exec":
            let command = try needed(keys.string("command"))
            let params = keys.value("params") ?? [:]
            call = { try await $0.exec(command, params: params) }
        case "run":
            let commands = try steps(of: &keys)
            call = { try await $0.run(commands) }
        case "props":
            let layer = try needed(keys.reference("layer"))
            let comp = try keys.reference("comp")
            let flat = try keys.flag("flat") ?? false
            let time = try keys.number("time")
            let depth = try keys.whole("depth")
            call = { try await $0.props(layer: layer, comp: comp, flat: flat, time: time, depth: depth) }
        case "get":
            let path = try needed(keys.string("path"))
            let layer = try needed(keys.reference("layer"))
            let comp = try keys.reference("comp")
            let time = try keys.number("time")
            call = { try await $0.get(path: path, layer: layer, comp: comp, at: time) }
        case "set":
            let path = try needed(keys.string("path"))
            let value = keys.value("value")
            let expression = try keys.string("expression")
            let layer = try needed(keys.reference("layer"))
            let comp = try keys.reference("comp")
            let time = try keys.number("time")
            call = {
                try await $0.set(path, to: value, expression: expression, layer: layer, comp: comp, at: time)
            }
        case "replay":
            let script = try needed(keys.string("script"))
            let keepGoing = try keys.flag("keepGoing") ?? false
            call = { try await $0.replay(script, keepGoing: keepGoing) }
        case "timings":
            let comp = try keys.reference("comp")
            let time = try keys.number("time")
            let scale = try keys.number("scale")
            let runs = try keys.whole("runs")
            let play = try keys.whole("play")
            let gpu = try keys.flag("gpu") ?? false
            call = {
                try await $0.timings(comp: comp, time: time, scale: scale, runs: runs, play: play, gpu: gpu)
            }
        case "previewRequest":
            let comp = try keys.reference("comp")
            let time = try keys.number("time")
            let width = try needed(keys.whole("width"))
            let height = try needed(keys.whole("height"))
            call = {
                // The typed call answers the values apart; the door answers one document holding them.
                let answer = try await $0.previewRequest(
                    comp, at: time.map { .seconds($0) }, width: width, height: height)
                return [
                    "request": answer.request,
                    "requestText": .string(answer.requestText),
                    "contentWidth": .number(Double(answer.contentWidth)),
                    "contentHeight": .number(Double(answer.contentHeight)),
                ]
            }
        default:
            throw Unspelt()
        }
        try keys.taken()
        return call
    }
}

/// What a typed call that starts a job takes: the engine, and what hears the job's progress.
typealias TypedJob = @Sendable (EffectsEngine, @escaping ProgressHandler) async throws -> ExportJob

/// Where an export goes and what else its request says, as `export` and `templates.render` take them: the
/// file, the format's word, and the request's other fields.
private func output(
    of keys: inout Keys
) throws -> (to: URL, format: EffectsEngine.RenderQueueCommands.AddFormat?, extra: [String: JSONValue]) {
    let to = URL(filePath: try needed(keys.string("output")))
    let format = try keys.string("format").map(EffectsEngine.RenderQueueCommands.AddFormat.init(rawValue:))
    return (to, format, keys.rest())
}

/// The typed call that starts the method `method` with `params` as a job, or nil when no typed call spells
/// it.
func typedJob(_ method: String, _ params: JSONValue) -> TypedJob? {
    spelt {
        var keys = try Keys(params)
        switch method {
        case "export":
            let comp = try keys.reference("comp")
            let (to, format, extra) = try output(of: &keys)
            return { try await $0.export(comp, to: to, format: format, progress: $1, extra: extra) }
        case "template.render":
            let template = URL(filePath: try needed(keys.string("template")))
            let values = try keys.object("values") ?? [:]
            var inner = try Keys(.object(try needed(keys.object("output"))))
            let (to, format, extra) = try output(of: &inner)
            try keys.taken()
            return {
                try await $0.templates.render(
                    template, values: values, to: to, format: format, progress: $1, extra: extra)
            }
        default:
            throw Unspelt()
        }
    }
}

/// The arguments of the typed `frame` that spell the frame request `params`, or nil when it names a key the
/// typed call has no argument for, as a frame of a template does.
struct TypedFrame: Sendable {
    let comp: ItemReference?
    let time: Double?
    let frame: Int?
    let maxSide: Double?
    let scale: Double?
    let transparent: Bool

    init?(_ params: JSONValue) {
        let arguments = spelt {
            var keys = try Keys(params)
            let named = (
                try keys.reference("comp"), try keys.number("time"), try keys.whole("frame"),
                try keys.number("maxSide"), try keys.number("scale"), try keys.flag("transparent") ?? false
            )
            try keys.taken()
            return named
        }
        guard let arguments else { return nil }
        (comp, time, frame, maxSide, scale, transparent) = arguments
    }
}
