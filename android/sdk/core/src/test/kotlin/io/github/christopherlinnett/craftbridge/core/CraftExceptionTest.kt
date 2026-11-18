// What a call answers with: one exception per row of the plan's table, carrying the core's own sentence.

package io.github.christopherlinnett.craftbridge.core

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import io.github.christopherlinnett.craftbridge.ffi.core.CraftException as BoundaryException

/**
 * The rows only: which exception each boundary variant becomes and which fields it keeps. The sentence
 * is the core's, which only the native library can give, so a host test hands one in and sees that it
 * is the message as handed; the words themselves are checked on a device, where the library loads.
 */
class CraftExceptionTest {
    private val words = "the core's sentence, whatever it is"

    @Test
    fun refusedParametersKeepTheWordsThatRefusedThem() {
        val refused = BoundaryException.InvalidParams("comp.new: unknown parameter(s) widht").asCoreException(words)
        assertIs<InvalidParamsException>(refused)
        assertEquals("comp.new: unknown parameter(s) widht", refused.detail)
        assertEquals(words, refused.message)
    }

    @Test
    fun aFailedCommandKeepsItsNameItsReasonAndItsStep() {
        val refused = BoundaryException.CommandFailed("effect.apply", """no effect "Gausian Blur"""", 3u).asCoreException(words)
        assertIs<CommandFailedException>(refused)
        assertEquals("effect.apply", refused.command)
        assertEquals("""no effect "Gausian Blur"""", refused.detail)
        assertEquals(3u, refused.step)
        assertEquals(words, refused.message)
    }

    @Test
    fun aFailedCommandHasNoStepWhenItWasNotARun() {
        val refused = BoundaryException.CommandFailed("effect.apply", "no effect", null).asCoreException(words)
        assertIs<CommandFailedException>(refused)
        assertEquals(null, refused.step)
    }

    @Test
    fun aMissingDocumentObjectIsNotFound() {
        val refused = BoundaryException.NotFound("""no layer "#9" in "Birthday Card"""").asCoreException(words)
        assertIs<NotFoundException>(refused)
        assertEquals("""no layer "#9" in "Birthday Card"""", refused.detail)
        assertEquals(words, refused.message)
    }

    @Test
    fun aCapabilityThisBuildLacksIsUnsupported() {
        val refused = BoundaryException.Unsupported("`export` does not run with progress in this app").asCoreException(words)
        assertIs<UnsupportedException>(refused)
        assertEquals("`export` does not run with progress in this app", refused.detail)
        assertEquals(words, refused.message)
    }

    @Test
    fun aCallTheAppStoppedIsCancelled() {
        val stopped = BoundaryException.Cancelled().asCoreException(words)
        assertIs<CancelledException>(stopped)
        assertEquals(words, stopped.message)
    }

    @Test
    fun aFolderTheEngineMayNotWriteInIsAnIoFailure() {
        val folder = "the config folder /outside/config is outside the container /data: the engine writes only inside its container"
        val refused = BoundaryException.Io(folder).asCoreException(words)
        assertIs<IoException>(refused)
        assertEquals(folder, refused.detail)
        assertEquals(words, refused.message)
    }

    @Test
    fun aDeviceThatWouldNotBeMadeIsAGpuFailure() {
        val refused = BoundaryException.Gpu("no Vulkan device").asCoreException(words)
        assertIs<GpuException>(refused)
        assertEquals("no Vulkan device", refused.detail)
        assertEquals(words, refused.message)
    }

    @Test
    fun aFailureWithNoRowOfItsOwnIsInternal() {
        val refused = BoundaryException.Internal("the actor call blew up").asCoreException(words)
        assertIs<InternalErrorException>(refused)
        assertEquals("the actor call blew up", refused.detail)
        assertEquals(words, refused.message)
    }
}
