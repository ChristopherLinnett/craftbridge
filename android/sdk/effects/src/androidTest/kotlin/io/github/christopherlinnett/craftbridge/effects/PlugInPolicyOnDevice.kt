// T24 on Android (plan §28, PD13): the case the desktop runner cannot prove on a phone. The corpus's two
// steps are what every platform answers alike. Where else a module loads from is each platform's own rule,
// and Android, like the desktop, loads what the confinement lets through: a module inside the app's
// container but outside the plug-in folder the app names loads, while a module outside every root the
// folders derive is the confinement's refusal, `Io`.

package io.github.christopherlinnett.craftbridge.effects

import android.content.Context
import io.github.christopherlinnett.craftbridge.core.IoException
import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.ffi.core.GpuChoice
import java.io.File
import java.util.UUID

/** The id the corpus's probe module carries, which each module written here replaces with an id of its own. */
private const val PROBE_ID = "conformance"

/**
 * The corpus's probe module, written to [path]: a module with [id] in place of the corpus's, of the same
 * length, since the module's manifest says how long it is, and a process registers one plug-in per id.
 */
private fun Context.writeProbe(
    id: String,
    path: File,
) {
    require(id.length == PROBE_ID.length) { "a module's id keeps the length its manifest says: ${id.length} is not ${PROBE_ID.length}" }
    val probe = assets.open("fixtures/probe.wat").use { it.readBytes().decodeToString() }
    path.parentFile?.mkdirs()
    path.writeText(probe.replace(PROBE_ID, id))
}

/** The plug-in checks of the platform's own rule: what loads from where. */
internal val plugInPolicyChecks: List<Case> =
    listOf(
        Case(
            "T24 on Android: a module in the container outside the plug-in folder loads, and one outside every root answers Io",
        ) { context ->
            val root = File(context.dataDir, "craftbridge-t24-${UUID.randomUUID()}")
            val container = File(root, "container")
            val inTheFolder = File(container, "plug-ins/probe.wasm")
            val inTheContainer = File(container, "loose/probe.wasm")
            val outside = File(root, "elsewhere/probe.wasm")
            context.writeProbe("foldprobe01", inTheFolder)
            context.writeProbe("looseprobe1", inTheContainer)
            context.writeProbe("outerprobe1", outside)
            val engine = EffectsEngine.start(context, engineFolders(container, "plug-ins"), GpuChoice.CPU_ONLY)
            try {
                suspend fun load(module: File): JSONValue = engine.exec("effect.plugins.load", mapOf("path" to JSONValue.Text(module.path)))
                // Without the folder's own module loading, a loader that refused everything would pass the refusal below.
                val control = load(inTheFolder).valueAt("/loaded/0/name")
                require(control == JSONValue.Text("Gate Probe")) { "the plug-in folder's module did not load: $control" }
                val loose = load(inTheContainer).valueAt("/loaded/0/name")
                require(
                    loose == JSONValue.Text("Gate Probe"),
                ) { "a module in the container, outside the plug-in folder, did not load: $loose" }
                val refused =
                    try {
                        load(outside)
                        null
                    } catch (refusal: IoException) {
                        refusal
                    }
                require(refused != null) { "a module outside every root must not load" }
                val words = "cannot read `${outside.path}`: it is outside the automation roots"
                require(refused.detail.startsWith(words)) { "a module outside every root was refused with ${refused.detail}, not $words" }
            } finally {
                engine.shutDown()
                root.deleteRecursively()
            }
        },
    )
