// What a scenario's own folder takes before a step's calls are made: the folders of the app the step runs
// in, the fixtures placed there, and the folder and the files' texts written into a request's parameters.
// Every runner of the corpus stages a scenario this way, so the staging is written once, beside the model.

import CraftBridge
import Foundation

/// Where the scenario's folder goes in a text of the corpus: the engine takes absolute paths, and the
/// folder is not known until the scenario runs.
public let containerMarker = "<container>"

/// The folders of an app whose container is `container`, and whose bundle is `bundle`, the app's read-only
/// root: the working folders sit inside the container, each hidden from a listing of it by a leading dot.
public func engineFolders(container: URL, bundle: URL?) -> Folders {
    func path(_ url: URL) -> String { url.path(percentEncoded: false) }
    let hidden = { (name: String) in path(container.appending(path: ".\(name)", directoryHint: .isDirectory)) }
    return Folders(
        container: path(container),
        bundle: bundle.map(path),
        config: hidden("config"),
        cache: hidden("cache"),
        models: hidden("models"),
        staged: hidden("staged"),
        plugins: nil,
        fonts: []
    )
}

extension Fixture {
    /// Copies the fixture from the corpus's folder into the scenario's folder `folder`, with the folder's
    /// path written wherever the file's text says `<container>` when the fixture says it names one.
    public func place(in folder: URL) throws {
        let source = Corpus.file.deletingLastPathComponent().appending(path: from)
        let placed = folder.appending(path: to)
        try FileManager.default.createDirectory(
            at: placed.deletingLastPathComponent(), withIntermediateDirectories: true)
        if namesContainer {
            let text = try String(contentsOf: source, encoding: .utf8)
            try text.replacingOccurrences(of: containerMarker, with: folder.path(percentEncoded: false))
                .write(to: placed, atomically: true, encoding: .utf8)
        } else {
            try FileManager.default.copyItem(at: source, to: placed)
        }
    }
}

extension JSONValue {
    /// This value with `text` written wherever a string holds `target`.
    public func replacing(_ target: String, with text: String) -> JSONValue {
        switch self {
        case .string(let value): .string(value.replacingOccurrences(of: target, with: text))
        case .array(let items): .array(items.map { $0.replacing(target, with: text) })
        case .object(let fields): .object(fields.mapValues { $0.replacing(target, with: text) })
        default: self
        }
    }

    /// The keys and indexes a pointer (RFC 6901) names, in order, each with its escapes read: none for the
    /// empty pointer, which names the whole document, and nil for a text that is no pointer, since every
    /// other pointer starts with a slash.
    public static func pointerTokens(_ pointer: String) -> [String]? {
        if pointer.isEmpty { return [] }
        guard pointer.hasPrefix("/") else { return nil }
        return pointer.dropFirst().split(separator: "/", omittingEmptySubsequences: false).map {
            $0.replacingOccurrences(of: "~1", with: "/").replacingOccurrences(of: "~0", with: "~")
        }
    }
}

extension Call {
    /// The parameters the request `method` is sent with from the scenario's folder `folder`: `params` with
    /// the folder's path written wherever they say `<container>`, and under each key of `files` the text
    /// `read` answers for the file that key names, which is the file's own unless the caller reads another
    /// way.
    ///
    /// - Throws: ``CraftError/InvalidParams(detail:)`` for parameters that take a file and are no object,
    ///   and ``CraftError/Io(detail:)`` for a file that does not read.
    public static func parameters(
        of method: String,
        _ params: JSONValue,
        files: [String: String],
        in folder: URL,
        reading read: (URL) throws -> String = { try String(contentsOf: $0, encoding: .utf8) }
    ) throws -> JSONValue {
        var sent = params.replacing(containerMarker, with: folder.path(percentEncoded: false))
        for (name, path) in files {
            guard case .object(var fields) = sent else {
                throw CraftError.InvalidParams(detail: "`\(method)`: parameters that take a file must be an object")
            }
            do {
                fields[name] = .string(try read(folder.appending(path: path)))
            } catch {
                throw CraftError.Io(detail: "cannot read \(path): \(error.localizedDescription)")
            }
            sent = .object(fields)
        }
        return sent
    }
}
