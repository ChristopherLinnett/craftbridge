// The template calls of the EffectCraft API. `templates.open` and `templates.render` extend the
// generated `templates` namespace rather than ``EffectsEngine``, so `engine.templates.open(…)` keeps the
// shape plan §5 writes: the engine's `templates` is the namespace, and a hand-written member of the engine
// with that name would stand beside it, which the compiler refuses only for a property or a call with no
// arguments and the `swift_build` stage refuses for every other.

import CraftBridge
import Foundation

extension EffectsEngine.TemplatesCommands {
    /// Opens the template at `template` as a live instance and answers it as a ``TemplateInstance``,
    /// through the engine's `template.open`. `values` sets controls as the instance is made, the same
    /// values `template.open` takes; ``TemplateInstance/set(_:_:)`` and
    /// ``TemplateInstance/set(_:media:)`` set them after the open. The answer carries the wrapper
    /// composition the instance plays in and the warnings of the fonts the phone lacks.
    public func open(_ template: URL, values: [String: JSONValue] = [:]) async throws -> TemplateInstance {
        let reply = try await engine.request(
            "template.open",
            given(["template": .string(template.path(percentEncoded: false)), "values": valuesKey(values)])
        )
        let answer = fields(of: reply)
        guard case .number(let value)? = answer["comp"], let id = Int(exactly: value) else {
            throw CraftError.Internal(detail: "`template.open`: the answer named no composition")
        }
        let comp = ItemReference.number(id)
        // `essential.set` names the layer a control sits on, and `template.open` answers only the
        // wrapper; the wrapper is the template's duplicate emptied of layers and holds exactly one,
        // the instance, so the wrapper's one layer is the instance's.
        let wrapper = fields(of: try await engine.comp.info(comp: comp.jsonValue))
        guard case .array(let layers)? = wrapper["layers"], case .object(let layer)? = layers.first,
            let instance = layer["id"]
        else {
            throw CraftError.Internal(detail: "`template.open`: the wrapper composition holds no instance")
        }
        var warnings: [JSONValue] = []
        if case .array(let items)? = answer["warnings"] {
            warnings = items
        }
        return TemplateInstance(engine: engine, comp: comp, instance: instance, warnings: warnings)
    }

    /// Opens the template at `template` afresh and exports the instance through the engine's
    /// `template.render`, answering the ``ExportJob`` that carries it: the run exports to `output` as
    /// `format`, which is a word the render queue knows a format by, since the render goes through the
    /// queue; leaving `format` out takes the word of `output`'s extension. `values` sets the controls
    /// as the instance is made, `progress` hears what the render reports, and `extra` carries the
    /// request's other fields the way `export` takes them; the engine checks every key. Unlike
    /// ``open(_:values:)`` a render names no composition of the session: its instance is fresh.
    public func render(
        _ template: URL,
        values: [String: JSONValue] = [:],
        to output: URL,
        format: EffectsEngine.RenderQueueCommands.AddFormat? = nil,
        progress: @escaping ProgressHandler = { _, _, _ in },
        extra: [String: JSONValue] = [:]
    ) async throws -> ExportJob {
        // A render names no composition: the engine refuses one, since the template renders an instance
        // of its own.
        let request = engine.exportRequest(comp: nil, to: output, format: format, extra: extra)
        return try ExportJob(
            engine: engine.handle,
            method: "template.render",
            params: given([
                "template": .string(template.path(percentEncoded: false)),
                "values": valuesKey(values),
                "output": request,
            ]),
            progress: progress
        )
    }
}

/// An opened template instance: the composition it plays in and the controls to set on it. The engine
/// keeps the instance in its session until the project is replaced.
public final class TemplateInstance: Sendable {
    private let engine: EffectsEngine

    /// The layer of the wrapper that holds the template, as `template.open` made it.
    private let instance: JSONValue

    /// The wrapper composition the instance plays in: what a preview or ``EffectsEngine/export(_:to:format:progress:extra:)``
    /// takes as the composition to show or export.
    public let comp: ItemReference

    /// The template's text layers whose font the phone lacks, as the open reported them: the engine
    /// substituted a fallback face for each and says nothing else.
    public let warnings: [JSONValue]

    init(engine: EffectsEngine, comp: ItemReference, instance: JSONValue, warnings: [JSONValue]) {
        self.engine = engine
        self.comp = comp
        self.instance = instance
        self.warnings = warnings
    }

    /// Sets the control `control` to `value`, the way the engine sets a control on the instance.
    public func set(_ control: String, _ value: JSONValue) async throws {
        _ = try await engine.essential.set(layer: instance, control: .name(control), value: value, extra: place)
    }

    /// Sets the media control `control` to the file at `media`: the engine imports the file and points
    /// the control at what it imported, the way a media value given to `template.open` is applied.
    public func set(_ control: String, media: URL) async throws {
        let path = media.path(percentEncoded: false)
        let reply = fields(of: try await engine.file.`import`(paths: [.string(path)]))
        if case .array(let errors)? = reply["errors"], let error = errors.first {
            let words = (try? error.jsonText()) ?? "the import's own refusal"
            throw CraftError.CommandFailed(
                command: "file.import",
                detail: "control `\(control)`: cannot import \(path): \(words)",
                step: nil
            )
        }
        guard case .array(let items)? = reply["items"], let item = items.first else {
            throw CraftError.CommandFailed(
                command: "file.import",
                detail: "control `\(control)`: \(path) did not import as footage",
                step: nil
            )
        }
        _ = try await engine.essential.set(layer: instance, control: .name(control), item: item, extra: place)
    }

    /// The composition the instance's layer is named in. A layer is named within a composition, and one
    /// left out is read in the session's active one, which the next open, or any call that opens another
    /// composition, changes: an instance named without it would set another card's control, or none.
    private var place: [String: JSONValue] {
        ["comp": comp.jsonValue]
    }
}

/// The `values` key of an open or a render: left out when no control is given a value, as the engine
/// reads a missing key.
private func valuesKey(_ values: [String: JSONValue]) -> JSONValue? {
    values.isEmpty ? nil : .object(values)
}

/// The fields an answer carries, or none when it is not an object: what names a reply by key never
/// refuses the call, and a reply without the name reads as the call not naming it.
private func fields(of reply: JSONValue) -> [String: JSONValue] {
    if case .object(let fields) = reply {
        return fields
    }
    return [:]
}
