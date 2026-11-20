// The calls the guides show that Postcard Studio itself never makes: an engine with plug-ins and more fonts,
// commands sent by their ids, and the font listing. Every Kotlin block of the guides is a run of a file this
// build compiles (`cargo xtask guide-samples`), the app's own calls quoted from the app's own files; these few
// sit in the debug source set, which the gate compiles, so the release build an app ships holds none of them.

package io.github.christopherlinnett.craftbridge.postcardstudio

import android.content.Context
import io.github.christopherlinnett.craftbridge.core.Folders
import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.core.jsonValue
import io.github.christopherlinnett.craftbridge.effects.EffectsEngine
import io.github.christopherlinnett.craftbridge.effects.RunStep
import io.github.christopherlinnett.craftbridge.effects.text
import java.io.File

/** The engine with WebAssembly plug-ins switched on, and one more font folder: what the app bundles. */
fun startEngineWithPluginsAndFonts(context: Context): EffectsEngine {
    val folders =
        Folders.appDefault(context).copy(
            plugins = File(context.filesDir, "PlugIns").path,
            fonts = listOf(File(context.filesDir, "Fonts").path),
        )
    return EffectsEngine.start(context, folders = folders)
}

/** Where EffectCraft's desktop would run a script, an app runs the engine's calls itself: one command
 * through [EffectsEngine.exec], with its parameters as JSON. */
suspend fun addTitle(
    engine: EffectsEngine,
    comp: JSONValue,
    title: String,
): JSONValue = engine.exec("essential.addText", mapOf("comp" to comp, "text" to title.jsonValue))

/** Several commands in succession, in order: a step that fails stops the run there, and the steps before it keep what they did. */
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

/** The faces an app may draw with: the phone's own font folder, the app's in `Folders.fonts`, and the engine's own. */
suspend fun faceListing(engine: EffectsEngine): JSONValue = engine.text.fonts()
