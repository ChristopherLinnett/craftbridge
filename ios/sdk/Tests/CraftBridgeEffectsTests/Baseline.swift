// The per-device baseline as Swift reads it: `conformance/effects/baseline.toml`, the one file the Kotlin
// check reads too, so the figures' names, the running example's size and the line a run prints are written once.
// The file's device records are the release script's to read, and checked here against what the file says a
// record holds.

import CraftBridge
import Foundation

/// What a run measures and prints, as `baseline.toml` defines it.
struct Baseline {
    /// What a figure that is a run's condition asks of the run: that it read the value the record's own runs read,
    /// or no more than a bound the file states once for every record.
    enum Condition: Equatable {
        case record
        case atMost(Double)
    }

    /// One figure of the line: its name, the decimals its value is printed with, whether a release whose figure is
    /// more than ``flagSlowerThanPercent`` above the baseline's is flagged, which a run's condition is not, and
    /// the platforms that measure it.
    struct Figure: Equatable {
        let name: String
        let decimals: Int
        let flagged: Bool
        var platforms: [String] = Baseline.platforms
        /// The platforms whose runs are recorded and reported for this figure but never flagged, because their
        /// own noise is wider than the allowance (`baseline.toml` says why).
        var unheldOn: [String] = []
        /// What a run must read of this figure for a release to be compared on it, when the figure is a condition.
        var condition: Condition?

        /// Whether a release's figure on `platform` is flagged when it is more than the allowance above the record's.
        func isHeld(on platform: String) -> Bool {
            flagged && platforms.contains(platform) && !unheldOn.contains(platform)
        }
    }

    /// The running example of plan §5, which every run builds, previews and exports.
    struct Example: Equatable {
        let width: Int
        let height: Int
        let seconds: Int
        let frameRate: Int
        let frames: Int
        let format: String
        let previewAt: Double
        let previewRepeats: Int
    }

    /// The word the line starts with.
    let line: String
    /// The word the run's warm preview repeats are printed under, after the figures.
    let repeatsWord: String
    /// The decimals each warm preview repeat is printed with.
    let repeatsDecimals: Int
    /// The percentage a release's figure may be above its baseline before the release notes flag it.
    let flagSlowerThanPercent: Double
    /// The fewest fresh runs a release's median, and a record, rests on.
    let comparisonRuns: Int
    let example: Example
    /// The figures, in the order the line prints them.
    let figures: [Figure]
    /// The device records the release script reads, as JSON.
    let devices: [JSONValue]

    /// The definition in `text`, refusing a table or a key the runner does not read.
    init(toml text: String) throws {
        guard case .object(let document) = try JSONValue(toml: text, of: "the baseline") else {
            throw CraftError.InvalidParams(detail: "the baseline is not a TOML table")
        }
        let known: Set<String> = [
            "line", "repeats_word", "repeats_decimals", "flag_slower_than_percent", "comparison", "running_example",
            "figure", "peak_memory", "device",
        ]
        if let unknown = document.keys.filter({ !known.contains($0) }).min() {
            throw CraftError.InvalidParams(detail: "the baseline holds the table `\(unknown)`, which no runner reads")
        }
        line = try Baseline.string(document["line"], "line")
        repeatsWord = try Baseline.string(document["repeats_word"], "repeats_word")
        repeatsDecimals = Int(try Baseline.number(document["repeats_decimals"], "repeats_decimals"))
        flagSlowerThanPercent = try Baseline.number(document["flag_slower_than_percent"], "flag_slower_than_percent")
        comparisonRuns = Int(
            try Baseline.number(try Baseline.table(document["comparison"], "comparison")["runs"], "comparison runs"))
        let example = try Baseline.table(document["running_example"], "running_example")
        self.example = Example(
            width: Int(try Baseline.number(example["width"], "width")),
            height: Int(try Baseline.number(example["height"], "height")),
            seconds: Int(try Baseline.number(example["seconds"], "seconds")),
            frameRate: Int(try Baseline.number(example["frame_rate"], "frame_rate")),
            frames: Int(try Baseline.number(example["frames"], "frames")),
            format: try Baseline.string(example["format"], "format"),
            previewAt: try Baseline.number(example["preview_at_seconds"], "preview_at_seconds"),
            previewRepeats: Int(try Baseline.number(example["preview_repeats"], "preview_repeats")))
        guard case .array(let listed)? = document["figure"], !listed.isEmpty else {
            throw CraftError.InvalidParams(detail: "the baseline defines no figure")
        }
        figures = try listed.map { entry in
            let table = try Baseline.table(entry, "figure")
            let name = try Baseline.string(table["name"], "name")
            // A key spelt otherwise, `unheld_on` above all, would be read as absent: a figure meant to be reported
            // only would be flagged on every release.
            if let unknown = table.keys.filter({ !Baseline.figureWords.contains($0) }).min() {
                throw CraftError.InvalidParams(
                    detail: "the figure `\(name)` holds the key `\(unknown)`, which no figure has")
            }
            let platforms = try Baseline.platformList(table["platforms"], "platforms", of: name) ?? Baseline.platforms
            let unheld = try Baseline.platformList(table["unheld_on"], "unheld_on", of: name) ?? []
            let flagged = try Baseline.flag(table["flagged"], "flagged")
            if let outside = unheld.first(where: { !platforms.contains($0) }) {
                throw CraftError.InvalidParams(
                    detail: "the figure `\(name)` is unheld on `\(outside)`, which does not measure it")
            }
            if !unheld.isEmpty && !flagged {
                throw CraftError.InvalidParams(detail: "the figure `\(name)` is unheld on a platform and never flagged")
            }
            // A figure no platform holds is one the rule never acts on, which `flagged = false` says once.
            if !unheld.isEmpty && Set(platforms).isSubset(of: unheld) {
                throw CraftError.InvalidParams(
                    detail: "the figure `\(name)` is unheld everywhere it is measured, which `flagged = false` says")
            }
            return Figure(
                name: name,
                decimals: Int(try Baseline.number(table["decimals"], "decimals")),
                flagged: flagged,
                platforms: platforms,
                unheldOn: unheld,
                condition: try Baseline.condition(of: table, named: name, flagged: flagged))
        }
        if case .array(let held)? = document["device"] { devices = held } else { devices = [] }
    }

    /// The baseline the repository holds, or the copy inside the app on a phone.
    static func current() throws -> Baseline {
        try Baseline(toml: try String(contentsOf: conformanceFile("baseline.toml"), encoding: .utf8))
    }

    /// The figures a run on `platform` measures, in the order the line prints them.
    func figures(on platform: String) -> [Figure] {
        figures.filter { $0.platforms.contains(platform) }
    }

    /// The line a run on iOS prints for `values`, one for each figure it measures by name and no other, in the
    /// order the file gives, then every one of the run's warm preview `repeats` in the order they ran.
    func line(for values: [String: Double], repeats: [Double]) throws -> String {
        let measured = figures(on: Baseline.platform)
        guard Set(values.keys) == Set(measured.map(\.name)) else {
            throw CraftError.InvalidParams(
                detail: "the run measured \(values.keys.sorted()), and the baseline names \(measured.map(\.name))")
        }
        guard repeats.count == example.previewRepeats else {
            throw CraftError.InvalidParams(
                detail: "the run made \(repeats.count) warm previews, "
                    + "and the running example makes \(example.previewRepeats)")
        }
        let words = measured.map { figure in
            "\(figure.name)=\(String(format: "%.\(figure.decimals)f", values[figure.name] ?? 0))"
        }
        let said = repeats.map { String(format: "%.\(repeatsDecimals)f", $0) }.joined(separator: ",")
        return ([line] + words + ["\(repeatsWord)=\(said)"]).joined(separator: " ")
    }

    /// What is wrong with the device records the baseline holds: each must say what it was measured on and under,
    /// rest on at least ``comparisonRuns`` runs and, for every figure its platform measures, hold the value of each
    /// run, with the median and the range those values give.
    func recordProblems() -> [String] {
        var problems: [String] = []
        var seen: Set<String> = []
        for (index, device) in devices.enumerated() {
            guard case .object(let record) = device else {
                problems.append("device \(index + 1) is not a table")
                continue
            }
            let id = record["id"].flatMap { if case .string(let id) = $0 { id } else { nil } } ?? "device \(index + 1)"
            if !seen.insert(id).inserted { problems.append("\(id): a second record has this id") }
            var platform = ""
            if case .string(let named)? = record["platform"] {
                platform = named
                if !Baseline.platforms.contains(named) {
                    problems.append("\(id): the platform `\(named)` is neither ios nor android")
                }
            }
            let measured = figures(on: platform)
            let known = Set(Baseline.recordWords + measured.map(\.name))
            for key in record.keys.sorted() where !known.contains(key) {
                problems.append("\(id): the key `\(key)` is not read")
            }
            for key in Baseline.recordWords where record[key] == nil { problems.append("\(id): no `\(key)`") }
            var runs = 0
            if case .number(let held)? = record["runs"] {
                runs = Int(held)
                if runs < comparisonRuns {
                    problems.append("\(id): \(runs) runs, fewer than the \(comparisonRuns) a baseline rests on")
                }
            }
            for figure in measured {
                problems += Baseline.spreadProblems(of: figure, in: record[figure.name], runs: runs, record: id)
            }
            // A record that names no condition would compare a release on every run, the throttled ones too.
            if Baseline.platforms.contains(platform) && !measured.contains(where: { $0.condition != nil }) {
                problems.append(
                    "\(id): no figure of \(platform) is a condition, "
                        + "so a release's runs made in other conditions would be compared with it")
            }
        }
        return problems
    }

    /// What is wrong with one figure's spread in a record: it holds the value of each of the record's runs, and
    /// the median and the range of those values, which a cost must have above zero.
    private static func spreadProblems(of figure: Figure, in value: JSONValue?, runs: Int, record id: String)
        -> [String]
    {
        guard case .object(let spread)? = value,
            case .number(let median)? = spread["median"], case .number(let low)? = spread["min"],
            case .number(let high)? = spread["max"], case .array(let listed)? = spread["values"], spread.count == 4
        else {
            return ["\(id): `\(figure.name)` is not a table of a median, a min, a max and the values of its runs"]
        }
        let values = listed.compactMap { value -> Double? in
            if case .number(let number) = value { number } else { nil }
        }
        guard values.count == listed.count, values.count == runs else {
            return ["\(id): `\(figure.name)` holds \(listed.count) values for \(runs) runs"]
        }
        let ends = (values.min() ?? 0, values.max() ?? 0)
        var problems: [String] = []
        if !(figure.flagged ? 0 < ends.0 : 0 <= ends.0) {
            problems.append("\(id): `\(figure.name)` has a value that is no measurement: \(ends.0)")
        }
        let held = [median, low, high]
        let derived = [Baseline.median(of: values), ends.0, ends.1]
        if zip(held, derived).contains(where: { abs($0 - $1) > 1e-6 * max(1, abs($1)) }) {
            problems.append(
                "\(id): `\(figure.name)` says median, min and max \(held), and its values give \(derived)")
        }
        // A record is compared under the condition its own runs were made in, so a run of it outside the condition,
        // or runs of it that read two values of what a release must match, leave nothing to compare a release with.
        switch figure.condition {
        case .record where ends.0 != ends.1:
            problems.append(
                "\(id): `\(figure.name)` ran from \(ends.0) to \(ends.1), "
                    + "and a record's runs read one value of it, which a release is compared on")
        case .atMost(let bound) where ends.1 > bound:
            problems.append(
                "\(id): `\(figure.name)` ran from \(ends.0) to \(ends.1), "
                    + "outside its own condition (at most \(bound))")
        default:
            break
        }
        return problems
    }

    /// The median of `values`: the middle one, or the mean of the two in the middle.
    static func median(of values: [Double]) -> Double {
        let sorted = values.sorted()
        let middle = sorted.count / 2
        guard sorted.indices.contains(middle) else { return 0 }
        return sorted.count % 2 == 1 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2
    }

    /// The platforms the baseline is measured on.
    static let platforms = ["ios", "android"]
    /// The platform this run measures.
    static let platform = "ios"
    /// The words of a record besides its figures: what it was measured on, how, and what the phone was doing.
    static let recordWords = ["id", "platform", "device", "os", "build", "conditions", "recorded", "runs"]
    /// The keys of a figure's table.
    static let figureWords = [
        "name", "decimals", "flagged", "platforms", "unheld_on", "condition", "condition_bound", "what",
    ]

    /// What a figure says of being a run's condition: `condition = "record"` asks a run to read what the record's runs
    /// read, `condition = "at_most"` with a `condition_bound` asks it to read no more than the bound, and a figure with
    /// neither is a cost or a plain report. A condition is never a cost, which `flagged` says of the costs.
    private static func condition(of table: [String: JSONValue], named name: String, flagged: Bool) throws
        -> Condition?
    {
        guard let kind = table["condition"] else {
            if table["condition_bound"] != nil {
                throw CraftError.InvalidParams(
                    detail: "the figure `\(name)` holds a `condition_bound` and no `condition`")
            }
            return nil
        }
        if flagged {
            throw CraftError.InvalidParams(
                detail: "the figure `\(name)` is a condition and flagged, which only a cost is")
        }
        let bound = table["condition_bound"]
        switch kind {
        case .string("record"):
            if bound != nil {
                throw CraftError.InvalidParams(
                    detail: "the figure `\(name)` is `record` and holds a `condition_bound`, which it has no use for")
            }
            return .record
        case .string("at_most"):
            guard case .number(let limit)? = bound else {
                throw CraftError.InvalidParams(
                    detail: "the figure `\(name)` is `at_most` and names no number `condition_bound`")
            }
            return .atMost(limit)
        case .string(let other):
            throw CraftError.InvalidParams(
                detail: "the figure `\(name)`'s `condition` is `\(other)`, which is neither record nor at_most")
        default:
            throw CraftError.InvalidParams(
                detail: "the figure `\(name)`'s `condition` is not text, which is neither record nor at_most")
        }
    }

    /// The platforms a figure's `key` lists, nil when the figure has no such key. A platform spelt otherwise would
    /// leave the figure out of every run's line and every record unseen, or flag it where it is to be reported only.
    private static func platformList(_ value: JSONValue?, _ key: String, of figure: String) throws -> [String]? {
        guard let value else { return nil }
        guard case .array(let listed) = value else {
            throw CraftError.InvalidParams(detail: "the figure `\(figure)`'s `\(key)` is not a list of platforms")
        }
        let named = try listed.map { try Baseline.string($0, key) }
        if let unknown = named.first(where: { !Baseline.platforms.contains($0) }) {
            throw CraftError.InvalidParams(
                detail: "the figure `\(figure)` names the platform `\(unknown)`, which is neither ios nor android")
        }
        return named
    }

    private static func string(_ value: JSONValue?, _ key: String) throws -> String {
        guard case .string(let text)? = value else {
            throw CraftError.InvalidParams(detail: "the baseline has no text `\(key)`")
        }
        return text
    }

    private static func flag(_ value: JSONValue?, _ key: String) throws -> Bool {
        guard case .bool(let flag)? = value else {
            throw CraftError.InvalidParams(detail: "the baseline has no true or false `\(key)`")
        }
        return flag
    }

    private static func number(_ value: JSONValue?, _ key: String) throws -> Double {
        guard case .number(let number)? = value else {
            throw CraftError.InvalidParams(detail: "the baseline has no number `\(key)`")
        }
        return number
    }

    private static func table(_ value: JSONValue?, _ key: String) throws -> [String: JSONValue] {
        guard case .object(let table)? = value else {
            throw CraftError.InvalidParams(detail: "the baseline has no table `\(key)`")
        }
        return table
    }
}
