# Integrating CraftBridge into an Android app

CraftBridge is an Android SDK that puts EffectCraft's rendering engine, made by the ArtCraft team, inside an app. One engine holds one document and takes the commands the desktop app's engine takes, bar those the phone's confined engine refuses, every command that runs a script among them; the SDK wraps them in Kotlin calls that suspend and throw `CraftException`. Every Kotlin block in this guide is a run of lines of a file the Postcard Studio sample under `android/samples/postcard-studio` compiles: the app's own code wherever the app makes the call, and `GuideSnippets.kt`, in the sample's debug source set alone, for the few calls the app never makes. The gate builds the sample against the SDK and refuses a guide block that is no such run, so what you read here builds.

The sample runs the whole loop for a birthday card: open a template, fill its two controls, preview the card, export an MP4. This guide follows the same order. For exports that must survive the app leaving the foreground, see [background-exports.md](background-exports.md); for every command and key, see the [generated command reference](generated/commands.md).

## What you need

- `minSdk` 29 (Android 10) or later; compile against SDK 37.
- Kotlin with coroutine support: the engine's calls are `suspend` functions, called from your coroutine scope.
- The `:core` and `:effects` modules of this repository's Gradle build, `android/sdk`. Until releases are published, an app takes that build as the sample does, as an included build that answers the SDK's Maven coordinates with its own modules, once `tools/build-android.sh` has built the native library into the module that packages it, which git never holds:

```gradle.kts
// settings.gradle.kts, for an app that vendors the repository:
includeBuild("vendor/craftbridge/android/sdk") {
    dependencySubstitution {
        substitute(module("io.github.christopherlinnett.craftbridge:core")).using(project(":core"))
        substitute(module("io.github.christopherlinnett.craftbridge:effects")).using(project(":effects"))
    }
}
```

```gradle.kts
// the app module's dependencies:
implementation("io.github.christopherlinnett.craftbridge:effects")
```

`:core` is the universal half — folders, JSON values, errors, preview and export plumbing. `:effects` is the EffectCraft plugin: the commands, templates, and the app-facing `EffectsEngine`. An app depends on `:effects`, which carries `:core` and the native library with it. The sample also includes the repository's own convention plugins, `android/sdk/build-logic`, which hold its code to the SDK's own checks; an app needs none of them.

## Start the engine

One engine per document, started when the app needs it and shut down when the last user of it is done. `EffectsEngine.start` proves it can write where the folders point and loads what they hold — fonts, models, staged copies — and removes whatever an export the system stopped last time left behind (see [background-exports.md](background-exports.md)). It is no `suspend` function: it waits for all of that, so call it from an IO context, never the main thread. The sample opens its one engine and card once, through `OpenedOnce`, which copies the template out of the app, starts the engine, opens the card in it, and ends the engine again when the card does not open:

```kotlin
private val opened =
    OpenedOnce(
        template = { birthdayCardTemplate(context) },
        start = { EffectsEngine.start(context) },
        open = { engine: EffectsEngine, template: File -> engine.templates.open(template.path) },
        shutDown = { engine: EffectsEngine -> engine.shutDown() },
        onEndFailure = { _: EffectsEngine, failure: Exception ->
            Log.w(LOG_TAG, "an engine whose card did not open could not be ended", failure)
        },
    )
```

and every call the app makes reaches the card through one door, which does that work on the IO threads:

```kotlin
private suspend fun card(): Opened<EffectsEngine, TemplateInstance> = withContext(Dispatchers.IO) { opened.get() }
```

The default `Folders.appDefault(context)` is the layout the SDK recommends: settings in the app's files, installed models and staged copies of imported media in the files Android keeps but never backs up, and what can be thrown away in the app's cache, which the system may empty. The start asks for the GPU unless `gpu` says otherwise, and throws `CraftException` rather than crashing on anything wrong with the folders.

### Plug-ins and extra fonts

WebAssembly plug-ins stay switched off until the app names the folder that holds them, so an app decides whether third-party effects run in it. Fonts join the phone's own families through `Folders.fonts`. `Folders` is a data class, so an app starts from `appDefault` and copies the two fields it wants:

```kotlin
fun startEngineWithPluginsAndFonts(context: Context): EffectsEngine {
    val folders =
        Folders.appDefault(context).copy(
            plugins = File(context.filesDir, "PlugIns").path,
            fonts = listOf(File(context.filesDir, "Fonts").path),
        )
    return EffectsEngine.start(context, folders = folders)
}
```

Everything the engine registers stays drawable for the process's life, fallback included; the `fonts` listing is what the app is shown (see [Fonts](#fonts)).

## Open a template

A `.ectemplate` file is a Craft document that names its editable controls. `templates.open` makes a live instance of one in the engine, from a file: the SDK hands the engine files, so a template the app ships among its assets is copied out to the app's files first, as the sample's `birthdayCardTemplate` does. Setting a control is one call — a text control takes a value, a media control a file:

```kotlin
/** Puts [name] on the card's Name control. */
suspend fun setName(name: String) {
    card().card.set(NAME_CONTROL, name.jsonValue)
}

/** Puts the picture at [photo] on the card's Photo control. */
suspend fun setPhoto(photo: File) {
    card().card.set(PHOTO_CONTROL, photo)
}
```

`TemplateInstance` carries `comp`, the composition the card plays in — everything downstream names it. A template written for families the phone lacks opens anyway, its `warnings` naming them; export and preview still run, drawing those faces with the fallback.

## Preview

`preview` draws one frame of the composition, at the moment it names, into a `Surface` the app owns — a `SurfaceView`'s or a `TextureView`'s. The first call attaches the surface, and the engine keeps it until the preview ends, so naming the same surface on every call costs no second attach; naming it again after a resize is how the resize reaches the frame. The sample's preview and its end:

```kotlin
suspend fun preview(
    surface: Surface,
    second: Double,
) {
    val studio = card()
    studio.engine.preview(surface, comp = studio.card.comp, time = second.seconds)
}

/** Lets the preview's surface go, when its view is. */
suspend fun endPreview() {
    opened.engine?.endPreview()
}
```

When the view goes, end the preview, and attach a fresh surface when it comes back — a draw onto a surface that has gone answers `GpuException`, and the next draw onto a fresh one draws again.

## Export

`export` renders the composition to a movie file, reporting progress as its frames are made, and returns when the movie is written — it is a call that waits, so a long export belongs to [the background recipe](background-exports.md). Cancel it by cancelling the coroutine that awaits it:

```kotlin
suspend fun exportMovie(
    movie: File,
    onProgress: ProgressHandler,
) {
    val studio = card()
    movie.parentFile?.mkdirs()
    studio.engine.export(
        request =
            mapOf<String, JSONValue>(
                "comp" to studio.card.comp,
                "output" to movie.path.jsonValue,
                "format" to "h264".jsonValue,
            ),
        onProgress = onProgress,
    )
}
```

`onProgress` hears frames done, frames total, and — when the engine can estimate it — the seconds left. A cancelled export leaves no file: the engine deletes what it had written before the call ends as a cancelled coroutine. A failed export may leave what it wrote, and one the system kills leaves its partial movie until the engine's next start removes it ([background-exports.md](background-exports.md)), so the movie at the output is whole once `export` has returned, and only then.

## Commands without a wrapper

EffectCraft's desktop runs scripts; the phone build never does — the confined engine refuses every command that runs one. What an app does instead is run the engine's calls in succession: `exec` sends one command by its id, and `run` sends a list:

```kotlin
suspend fun addTitle(
    engine: EffectsEngine,
    comp: JSONValue,
    title: String,
): JSONValue = engine.exec("essential.addText", mapOf("comp" to comp, "text" to title.jsonValue))
```

```kotlin
suspend fun addTitleAndCaption(
    engine: EffectsEngine,
    comp: JSONValue,
    title: String,
    caption: String,
): JSONValue =
    engine.run(
        listOf(
            RunStep(command = "essential.addText", params = mapOf("comp" to comp, "text" to title.jsonValue)),
            RunStep(command = "essential.addText", params = mapOf("comp" to comp, "text" to caption.jsonValue)),
        ),
    )
```

`run` reads every step before it runs the first, so a malformed step anywhere runs none of them; it then runs the steps in order and stops at the first that fails, throwing `CommandFailedException` naming that step, while the steps before it keep what they did. Values in and out are `JSONValue`, whose cases mirror JSON; `"text".jsonValue` and `number.jsonValue` lift Kotlin values into it. The [command reference](generated/commands.md) lists every command with its keys, types and requiredness.

## Fonts

`text.fonts` lists the families the engine has registered — the phone's own from the system's font folder, the ones the app bundles in `Folders.fonts`, and the engine's own — with a `count`:

```kotlin
suspend fun faceListing(engine: EffectsEngine): JSONValue = engine.text.fonts()
```

On Android the listing passes through as the engine composes it; iOS trims the same call to the faces its app should offer (see the [iOS guide](integration-ios.md#fonts)). Every registered face stays drawable through fallback whatever the listing shows.

## Errors

Everything the engine can refuse you with is a `CraftException`: bad parameters (`InvalidParamsException`), a command that failed (`CommandFailedException`, naming the step of a `run`), a missing item (`NotFoundException`), something this phone or build cannot do (`UnsupportedException`), disk trouble (`IoException`), GPU trouble (`GpuException`), and `InternalErrorException` for anything else — never a crash. A call stopped from elsewhere, as by `shutDown`, answers `CancelledException`; a call whose own coroutine was cancelled ends as that cancel, a `CancellationException` whose cause is the engine's answer, so the scope it runs in reads it as the cancel it is. Each exception's `message` is the engine's own words. The sample's export service tells a stopped export from a failed one:

```kotlin
} catch (stopped: CancellationException) {
    studio.exportIs(ExportState.Stopped)
    throw stopped
} catch (failure: CraftException) {
    studio.exportIs(ExportState.Failed(failure.message.orEmpty()))
```

## When the app leaves the foreground

An export longer than a few seconds needs a foreground service on Android — and, beside it, a partial wake lock: a service keeps the process alive but not the CPU awake. [background-exports.md](background-exports.md) holds the recipe.
