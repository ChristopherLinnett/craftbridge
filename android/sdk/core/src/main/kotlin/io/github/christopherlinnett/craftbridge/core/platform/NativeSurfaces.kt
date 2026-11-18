// The one JNI door of the library: an app's `Surface` into the native window the engine previews on.

package io.github.christopherlinnett.craftbridge.core

import android.view.Surface
import io.github.christopherlinnett.craftbridge.ffi.core.findLibraryName

/**
 * The core's bindings, whose lookup names the one native library the SDK ships (AGENTS.md §2) as the
 * bindgen wrote it from the layer table, and honours an app's override of it, as the bindings' own load does.
 */
private const val BINDINGS = "craftbridge_core_ffi"

/**
 * The native side of a preview surface. JNI finds a native function by the name spelled from its class's
 * package, so this object's name and package are the names the native library exports its two functions
 * under, and neither can change without the library changing with it.
 *
 * `acquire` turns a [Surface] into a window the native library holds under an id, which
 * `Engine.attachWindow` takes; `release` gives a window up that was never attached, or whose attach
 * failed. Releasing an id the attach already took, or one that was never handed out, does nothing, so a
 * caller releases whichever way the attach ended.
 *
 * `System.loadLibrary` is what makes these two functions findable: the JVM looks a native function up in
 * the libraries it loaded itself, which is no part of JNA's own `dlopen` of the same file. The android
 * linker keeps one copy of a library whichever of the two loads it first.
 */
internal object NativeSurfaces {
    init {
        // The name the bindings' own lookup gives, and no copy of it: a distribution library of another
        // name renames the bindings' load and this one together.
        System.loadLibrary(findLibraryName(componentName = BINDINGS))
    }

    /**
     * The id of the window that names [surface], or zero when it is not a live surface.
     *
     * The window lives on in the native library until an attach takes it or [release] gives it up, so
     * the surface itself may end first.
     */
    @JvmStatic
    external fun acquire(surface: Surface): Long

    /** Give up the window [id] names, if the native library still holds it. */
    @JvmStatic
    external fun release(id: Long)
}
