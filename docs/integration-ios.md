# Integrating CraftBridge into an iOS app

CraftBridge is a Swift SDK that puts EffectCraft's rendering engine, made by the ArtCraft team, inside an iOS app. One engine holds one document and takes the commands the desktop app's engine takes, bar those the phone's confined engine refuses, every command that runs a script among them; the SDK wraps them in Swift calls that throw `CraftError` and report progress as they go. Every Swift block in this guide is a run of lines of a file the Postcard Studio sample under `ios/samples/PostcardStudio` compiles: the app's own code wherever the app makes the call, and `GuideSnippets.swift`, compiled into the sample's Debug build alone, for the few calls the app never makes. The gate builds the sample against the SDK and refuses a guide block that is no such run, so what you read here builds.

The sample runs the whole loop for a birthday card: open a template, fill its two controls, preview the card, export an MP4. This guide follows the same order. For exports that must survive the app leaving the foreground, see [background-exports.md](background-exports.md); for every command and key, see the [generated command reference](generated/commands.md).

## What you need

- iOS 17.0 or later — the package declares `.iOS(.v17)` and the sample targets it.
- Swift 6 language mode, an app that adopts structured concurrency — every engine call is `async throws`.
- The `CraftBridge` package, `ios/sdk`. Until releases are published an app takes it from this repository by its path, as the sample does, once `tools/build-xcframework.sh` has built the native library into the XCFramework the package's binary target links, which git never holds:

```text
// Package.swift, or the Xcode project's package list, for an app that vendors the repository:
.package(path: "vendor/craftbridge/ios/sdk")
// and then, for the target that renders (a path package is named by its folder):
.product(name: "CraftBridge", package: "sdk"),
.product(name: "CraftBridgeEffects", package: "sdk"),
```

`CraftBridge` is the universal core — folders, JSON values, errors, preview and export plumbing. `CraftBridgeEffects` is the EffectCraft plugin: the commands, templates, and the app-facing `EffectsEngine`. An app that renders links both and imports both, as the sample does: `Folders`, `JSONValue` and `CraftError` are the core's, and the plugin does not re-export them.

## Start the engine

One engine per document, started when the app needs it and shut down when the last user of it is done. `start` proves it can write where the folders point, loads what they hold — fonts, models, staged copies — and removes whatever an export the system stopped last time left behind (see [background-exports.md](background-exports.md)). The sample starts its engine in `Studio.start()`:

```swift
let engine = try await EffectsEngine.start(folders: .appDefault, gpu: .preferred)
```

`Folders.appDefault` is the layout the SDK recommends: settings, installed models and staged copies of imported media under `Library/Application Support/CraftBridge`, and what can be thrown away under `Library/Caches/CraftBridge`, which iOS may empty. `gpu: .preferred` asks for the GPU, and `.cpuOnly` for none. The call throws a `CraftError` rather than crashing on anything wrong with the folders — a full disk is `CraftError.Io`.

### Plug-ins and extra fonts

WebAssembly plug-ins stay switched off until the app names the folder that holds them, so an app decides whether third-party effects run in it:

```swift
func startEngineAllowingPlugins(in folder: String) async throws -> EffectsEngine {
    let folders = Folders.appDefault.allowingPlugins(in: folder)
    return try await EffectsEngine.start(folders: folders, gpu: .preferred)
}
```

The folder is named inside the app bundle: on iOS the engine loads a plug-in from no folder but one the app bundles. Fonts work the same way: the phone's own families are always registered, and whatever folder you add to `Folders.fonts` joins them. The SDK's bundled faces — the Noto Sans SC and Noto Sans TC subsets in `rust/effects/craftbridge-effects/assets/fonts`, with their licence — reach an app the same way, as a folder it bundles and names here:

```swift
func startEngineWithFonts(_ fonts: URL) async throws -> EffectsEngine {
    var folders = Folders.appDefault
    folders.fonts = [fonts.path]
    return try await EffectsEngine.start(folders: folders, gpu: .preferred)
}
```

Every face ever registered stays drawable for the process's life — fallback included; what the `fonts` listing shows is trimmed to what an app should offer (see [Fonts](#fonts)).

## Open a template

A `.ectemplate` file is a Craft document that names its editable controls. `templates.open` makes a live instance of one in the engine; the sample opens the Birthday Card its bundle carries:

```swift
guard let template = Bundle.main.url(forResource: "Birthday Card", withExtension: "ectemplate") else {
    throw StudioError.templateMissing
}
card = try await engine.templates.open(template)
```

Setting a control is one call: a text control takes a value, a media control a file. The sample sets each control again only when it has changed:

```swift
if appliedName != name {
    try await card.set("Name", .string(name))
    appliedName = name
}
if let photo, appliedPhoto != photo {
    try await card.set("Photo", media: photo)
    appliedPhoto = photo
}
```

`TemplateInstance` carries `comp`, the composition the card plays in — everything downstream names it. A template written for families the phone lacks opens anyway, its `warnings` naming them; export and preview still run, drawing those faces with the fallback.

## Preview

`preview` draws one frame of the composition, at the moment it names, into a `CAMetalLayer` the app owns, on the main actor. The first call attaches the layer, and the engine keeps it until the preview ends, so naming the same layer on every call costs no second attach. The sample draws the card whenever a control or the moment shown changes:

```swift
if let layer = previewLayer {
    try await engine.preview(card.comp, into: layer, at: .seconds(time))
}
```

When the view goes, end the preview, and attach a fresh layer when it comes back. Postcard Studio's one screen never goes, so the call is one of the snippets:

```swift
@MainActor
func stopPreview(_ engine: EffectsEngine) async throws {
    try await engine.endPreview()
}
```

By default the engine also lets its layer go when the app enters the background, and the next `preview` attaches it again (see [background-exports.md](background-exports.md)).

## Export

`export` starts a render of the composition to a movie file and answers an `ExportJob` at once — starting an export is never a wait. The sample's export, in `ExportRecipe.swift`, asks for the hardware video encoder where the phone has one through the `hardware` extra, and hands each progress report both to the system's background task and to the screen:

```swift
job = try await engine.export(
    comp, to: movie, format: .h264,
    progress: { done, total, _ in
        Task { @MainActor in
            systemTask?.report(done: done, total: total)
            if total > 0 { progress(Double(done) / Double(total)) }
        }
    },
    extra: ["hardware": true])
```

`progress` hears frames done, frames total, and — when the engine can estimate it — the seconds left. The job's `result` is the export's answer, awaited for as long as the export runs; cancelling the task that awaits it cancels the export:

```swift
_ = try await job.result
```

`cancel()` stops the job from anywhere, at the export's next progress report; the sample's stop also covers an export still starting:

```swift
static func cancel() {
    stopAsked = true
    running?.cancel()
}
```

A stopped export leaves no file: the engine deletes what it had written before `result` throws `CraftError.Cancelled`. A failed export may leave what it wrote, and one the system kills leaves its partial movie until the engine's next `start` removes it ([background-exports.md](background-exports.md)), so the movie at the output is whole once `result` has answered, and only then.

## Commands without a wrapper

EffectCraft's desktop runs scripts; the phone build never does — the confined engine refuses every command that runs one. What an app does instead is run the engine's calls in succession: `exec` sends one command by its id, and `run` sends a list:

```swift
func addTitle(_ engine: EffectsEngine, to comp: ItemReference, title: String) async throws -> JSONValue {
    try await engine.exec("essential.addText", params: ["comp": comp.jsonValue, "text": .string(title)])
}
```

```swift
func addTitleAndCaption(
    _ engine: EffectsEngine,
    to comp: ItemReference,
    title: String,
    caption: String
) async throws -> JSONValue {
    try await engine.run([
        ("essential.addText", ["comp": comp.jsonValue, "text": .string(title)]),
        ("essential.addText", ["comp": comp.jsonValue, "text": .string(caption)]),
    ])
}
```

`run` reads every step before it runs the first, so a malformed step anywhere runs none of them; it then runs the steps in order and stops at the first that fails, throwing `CraftError.CommandFailed` naming that step, while the steps before it keep what they did. Values in and out are `JSONValue`, whose cases mirror JSON. The [command reference](generated/commands.md) lists every command with its keys, types and requiredness.

## Fonts

`text.fonts` lists the families the app may draw with: the phone's own and the ones in its `Folders.fonts`, and nothing else — each with its styles and an origin of `system` or `bundled`, and a `count`:

```swift
func faceCount(_ engine: EffectsEngine) async throws -> Int {
    let listing = try await engine.text.fonts()
    guard case .object(let fields) = listing, case .number(let count)? = fields["count"] else { return 0 }
    return Int(count)
}
```

Every listing that reaches the app is trimmed this way: `exec`'s, each `run` step's, and each `text.fonts` result an `engine.batch` answers. Only code inside EffectCraft's own dispatch sees the full registry — an `engine.batch` step that refers to an earlier step's listing by `$N`, and `replay`'s `expect` steps. Drawing may use any registered face through fallback, whatever the listing shows.

## Errors

Everything the engine can refuse you with is a `CraftError`: bad parameters (`InvalidParams`), a command that failed (`CommandFailed`, naming the step of a `run`), a missing item (`NotFound`), something this phone or build cannot do (`Unsupported`), a stopped export (`Cancelled`), disk trouble (`Io`), GPU trouble (`Gpu`), and `Internal` for anything else — never a crash. Each carries the engine's own words as its `errorDescription`. The sample tells a stopped export from a failed one:

```swift
} catch CraftError.Cancelled {
    stage = .ready
    note = "The export was stopped. Export again to make the card."
    refreshSoon()
} catch {
    stage = .failed(describe(error))
}
```

and shows the engine's words for anything else:

```swift
private func describe(_ error: any Error) -> String {
    error.localizedDescription
}
```

## When the app leaves the foreground

An export longer than a few seconds needs the platform's background machinery. For iOS — continued processing tasks on iOS 26, the engine's own background task on 17–25, and recovery from a kill — read [background-exports.md](background-exports.md).
