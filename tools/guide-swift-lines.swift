// Which lines of a Swift file a shipping build compiles to nothing, for `cargo xtask
// guide-samples` (AGENTS.md §10): every non-blank character of a reported line sits in a
// comment, in a multi-line string literal's body, or in an `#if` branch no shipping build
// compiles. A guide block quoted wholly from such lines quotes prose dressed as code, and
// the check refuses it. Swift answers this through the compiler's own parser: this script
// reads each file with SwiftSyntax, the library swift-format and SourceKit-LSP read with,
// and weighs each `#if` condition with SwiftIfConfig, the evaluator that library gives the
// compiler's own rules for one, so no text scanner of the gate's own ever guesses at a
// string, a comment or a condition (§10). It runs on the swift-syntax the pinned Xcode's
// toolchain carries in `usr/lib/swift/host`, with the interfaces that copy ships for code
// built against it; a toolchain without one fails the run in the compiler's own words.
//
// A branch is dead only when no shipping build compiles it. The builds are the arguments
// `--ship <configuration>:<os>:<architecture>:<environment>:<conditions>`, one per
// configuration the gate builds on each slice the native library is built for, every fact
// read by the caller from the project's own settings and the toolchain's own slices. On
// each build the `#if`s are taken as the compiler takes them: the first clause whose
// condition holds is the one that compiles, an `#else` only when none before it held, and
// a nested `#if` only inside a clause that compiles. A condition no source can settle —
// `canImport`, `hasFeature`, `hasAttribute`, a language or compiler version — may hold, so
// it, and every clause after it, may compile: refusing a guide block over a maybe would
// punish real code.
//
// The answer is stdout, one line each: `S` and the file for a file read, then `I`, the file
// and the line for each of its lines that compile to nothing; `E`, the file and a reason
// for a file that could not be read.

import Foundation
import SwiftDiagnostics
import SwiftIfConfig
import SwiftParser
import SwiftSyntax

// One shipping build: one configuration's conditions on one slice. What no source can
// settle is no fact of the build's, so asking it throws, and the evaluator reports the
// condition unsettled.
struct Ship: BuildConfiguration {
    let conditions: Set<String>
    let os: String
    let architecture: String
    let environment: String
    // The version the build is weighed at; the caller weighs each condition at two, so a
    // condition that reads a version is told by the answers differing.
    let version: VersionTuple

    struct Unknowable: Error {}

    func isCustomConditionSet(name: String) throws -> Bool { conditions.contains(name) }
    func hasFeature(name: String) throws -> Bool { throw Unknowable() }
    func hasAttribute(name: String) throws -> Bool { throw Unknowable() }
    func canImport(
        importPath: [(TokenSyntax, String)],
        version: CanImportVersion
    ) throws -> Bool { throw Unknowable() }
    func isActiveTargetOS(name: String) throws -> Bool { name == os }
    func isActiveTargetArchitecture(name: String) throws -> Bool { name == architecture }
    func isActiveTargetEnvironment(name: String) throws -> Bool { name == environment }
    // Every slice is an Apple one: the Objective-C runtime, Mach-O, 64-bit and little-endian.
    func isActiveTargetRuntime(name: String) throws -> Bool { name == "_ObjC" }
    func isActiveTargetPointerAuthentication(name: String) throws -> Bool { throw Unknowable() }
    func isActiveTargetObjectFormat(name: String) throws -> Bool { name == "MachO" }
    var targetPointerBitWidth: Int { 64 }
    var targetAtomicBitWidths: [Int] { [8, 16, 32, 64, 128] }
    var endianness: Endianness { .little }
    var languageVersion: VersionTuple { version }
    var compilerVersion: VersionTuple { version }
}

// What one condition says on one build: it holds, it does not, or no source can tell.
enum Answer {
    case holds
    case fails
    case unsettled
}

func answer(_ condition: ExprSyntax, on ship: Ship) -> Answer {
    func weigh(at version: VersionTuple) -> (Bool, Bool) {
        let at = Ship(
            conditions: ship.conditions,
            os: ship.os,
            architecture: ship.architecture,
            environment: ship.environment,
            version: version
        )
        let (state, _, diagnostics) = IfConfigRegionState.evaluating(condition, in: at)
        let unsettled = diagnostics.contains { $0.diagMessage.severity == .error }
        return (state == .active, unsettled)
    }
    let (low, lowUnsettled) = weigh(at: VersionTuple(1))
    let (high, highUnsettled) = weigh(at: VersionTuple(1_000))
    if lowUnsettled || highUnsettled || low != high {
        return .unsettled
    }
    return low ? .holds : .fails
}

// One mark per source byte; marks applied later overwrite earlier ones.
struct Marks {
    private var inert: [Bool]

    init(length: Int) {
        inert = [Bool](repeating: false, count: length)
    }

    mutating func mark(from: Int, to: Int, dead: Bool) {
        guard from >= 0, to <= inert.count, from < to else {
            return
        }
        for index in from..<to {
            inert[index] = dead
        }
    }

    // A line is inert when every byte of it heavier than a space — every non-blank
    // character, continuation bytes of a multi-byte one included — stands under a dead
    // mark. A blank line is no character's witness and answers not.
    func lineIsInert(_ text: [UInt8], from start: Int, to end: Int) -> Bool {
        var witnessed = false
        for index in start..<end where text[index] > UInt8(ascii: " ") {
            witnessed = true
            if !inert[index] {
                return false
            }
        }
        return witnessed
    }
}

// The string bodies: a literal that spans lines is a place prose hides, and the code
// interpolated into it compiles again, the deepest mark written last. A one-line literal is
// the code that uses it, raw or not.
final class StringBodies: SyntaxVisitor {
    var marks: Marks

    init(marks: Marks) {
        self.marks = marks
        super.init(viewMode: .sourceAccurate)
    }

    override func visit(_ node: StringLiteralExprSyntax) -> SyntaxVisitorContinueKind {
        if node.openingQuote.tokenKind == .multilineStringQuote {
            marks.mark(
                from: node.positionAfterSkippingLeadingTrivia.utf8Offset,
                to: node.endPositionBeforeTrailingTrivia.utf8Offset,
                dead: true
            )
        }
        return .visitChildren
    }

    override func visit(_ node: ExpressionSegmentSyntax) -> SyntaxVisitorContinueKind {
        // The interpolated expressions are the segment's own; the glue around them — the
        // backslash, the pounds, the parens — is the string's.
        marks.mark(
            from: node.expressions.positionAfterSkippingLeadingTrivia.utf8Offset,
            to: node.expressions.endPositionBeforeTrailingTrivia.utf8Offset,
            dead: false
        )
        return .visitChildren
    }
}

// The `#if` branches no ship compiles, each as the set of ships it compiles on: a clause
// compiles on a ship where the clause enclosing it does and the compiler would take it.
final class DeadBranches: SyntaxVisitor {
    var marks: Marks
    let ships: [Ship]
    private var enclosing: [Set<Int>]

    init(marks: Marks, ships: [Ship]) {
        self.marks = marks
        self.ships = ships
        enclosing = [Set(ships.indices)]
        super.init(viewMode: .sourceAccurate)
    }

    override func visit(_ node: IfConfigDeclSyntax) -> SyntaxVisitorContinueKind {
        let outer = enclosing.last ?? []
        var compiles: [Set<Int>] = Array(repeating: [], count: node.clauses.count)
        for ship in outer {
            // Once a clause holds, no later one compiles, even after clauses that only may
            // have held: one of them or this one is the clause the compiler takes.
            var taken = false
            for (index, clause) in node.clauses.enumerated() {
                let said = clause.condition.map { answer($0, on: ships[ship]) } ?? .holds
                if taken || said == .fails {
                    continue
                }
                compiles[index].insert(ship)
                taken = said == .holds
            }
        }
        if compiles.allSatisfy(\.isEmpty) && !outer.isEmpty {
            // No clause compiles anywhere it could: the directives compile away with their
            // branches, `#if` to `#endif`.
            marks.mark(
                from: node.positionAfterSkippingLeadingTrivia.utf8Offset,
                to: node.endPositionBeforeTrailingTrivia.utf8Offset,
                dead: true
            )
        }
        for (clause, ships) in zip(node.clauses, compiles) {
            if ships.isEmpty {
                // The branch's own code, whatever it holds — statements, a type's members,
                // cases, attributes — after its directive, whose line stays.
                let head =
                    clause.condition?.endPositionBeforeTrailingTrivia
                    ?? clause.poundKeyword.endPositionBeforeTrailingTrivia
                marks.mark(from: head.utf8Offset, to: clause.endPositionBeforeTrailingTrivia.utf8Offset, dead: true)
            }
            if let elements = clause.elements {
                enclosing.append(ships)
                walk(elements)
                enclosing.removeLast()
            }
        }
        return .skipChildren
    }
}

// Comments are trivia to SwiftSyntax, not tokens: they are read off every token's own
// trimming, each kind — line, block, and the two doc forms — dead where it sits, written
// over the strings' marks so a comment inside interpolated code is dead too. Trivia pieces
// carry a length but no position of their own, so the walk runs the offset forward from
// where the token's trimming starts by each piece's source length.
func markComments(in tree: some SyntaxProtocol, marks: inout Marks) {
    for token in tree.tokens(viewMode: .sourceAccurate) {
        var leading = token.position.utf8Offset
        for piece in token.leadingTrivia {
            markTrivia(piece, at: &leading, marks: &marks)
        }
        var trailing = token.endPositionBeforeTrailingTrivia.utf8Offset
        for piece in token.trailingTrivia {
            markTrivia(piece, at: &trailing, marks: &marks)
        }
    }
}

private func markTrivia(_ piece: Trivia.Element, at offset: inout Int, marks: inout Marks) {
    let length = piece.sourceLength.utf8Length
    switch piece {
    case .lineComment, .blockComment, .docLineComment, .docBlockComment:
        marks.mark(from: offset, to: offset + length, dead: true)
    default:
        break
    }
    offset += length
}

func answer(file: String, ships: [Ship]) {
    let text: [UInt8]
    do {
        text = [UInt8](try Data(contentsOf: URL(fileURLWithPath: file)))
    } catch {
        print("E\t\(file)\tthe file could not be read: \(error)")
        return
    }
    guard let source = String(bytes: text, encoding: .utf8) else {
        print("E\t\(file)\tthe file is not UTF-8")
        return
    }
    let tree = Parser.parse(source: source)
    let strings = StringBodies(marks: Marks(length: text.count))
    strings.walk(tree)
    var marks = strings.marks
    markComments(in: tree, marks: &marks)
    let branches = DeadBranches(marks: marks, ships: ships)
    branches.walk(tree)
    marks = branches.marks
    print("S\t\(file)")
    var lineStart = 0
    var line = 1
    for (offset, byte) in text.enumerated() where byte == UInt8(ascii: "\n") {
        if marks.lineIsInert(text, from: lineStart, to: offset) {
            print("I\t\(file)\t\(line)")
        }
        lineStart = offset + 1
        line += 1
    }
    if lineStart < text.count, marks.lineIsInert(text, from: lineStart, to: text.count) {
        print("I\t\(file)\t\(line)")
    }
}

var ships: [Ship] = []
var files: [String] = []
var arguments = CommandLine.arguments.dropFirst()
while let argument = arguments.popFirst() {
    guard argument == "--ship" else {
        files.append(argument)
        continue
    }
    let fields = (arguments.popFirst() ?? "").split(separator: ":", maxSplits: 4, omittingEmptySubsequences: false)
    guard fields.count == 5 else {
        print("E\t--ship\tnames no <configuration>:<os>:<architecture>:<environment>:<conditions>")
        exit(2)
    }
    ships.append(
        Ship(
            conditions: Set(fields[4].split(separator: ",").map(String.init)),
            os: String(fields[1]),
            architecture: String(fields[2]),
            environment: String(fields[3]),
            version: VersionTuple(1)
        )
    )
}
if ships.isEmpty {
    print("E\tguide-swift-lines\tno --ship names a shipping build")
    exit(2)
}
for file in files {
    answer(file: file, ships: ships)
}
