// The native library the JNI door loads is the one the core's bindings name, whatever it is called.

package io.github.christopherlinnett.craftbridge.core

import kotlin.test.Test
import kotlin.test.assertContains
import kotlin.test.assertFailsWith

class NativeLibraryNameTest {
    /**
     * The bindings' own override renames the library they load. The door's first use loads it, and the
     * host holds no library of that name, so the platform's refusal names the library the door asked for.
     */
    @Test
    fun theDoorLoadsTheLibraryTheBindingsNameEvenWhenTheyRenameIt() {
        val override = "uniffi.component.craftbridge_core_ffi.libraryOverride"
        System.setProperty(override, "renamed_ffi")
        try {
            val refused = assertFailsWith<UnsatisfiedLinkError> { NativeSurfaces.release(0) }
            assertContains(refused.message.orEmpty(), "renamed_ffi")
        } finally {
            System.clearProperty(override)
        }
    }
}
