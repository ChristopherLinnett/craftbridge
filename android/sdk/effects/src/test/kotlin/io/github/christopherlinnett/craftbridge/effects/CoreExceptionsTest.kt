// The core's failures as an app's own tests make them, from outside the module that defines them.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.CancelledException
import io.github.christopherlinnett.craftbridge.core.CommandFailedException
import io.github.christopherlinnett.craftbridge.core.CraftException
import io.github.christopherlinnett.craftbridge.core.GpuException
import io.github.christopherlinnett.craftbridge.core.InternalErrorException
import io.github.christopherlinnett.craftbridge.core.InvalidParamsException
import io.github.christopherlinnett.craftbridge.core.IoException
import io.github.christopherlinnett.craftbridge.core.NotFoundException
import io.github.christopherlinnett.craftbridge.core.UnsupportedException
import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * An app tests how its code handles each failure an engine can answer, without an engine, by making the
 * failure itself. This test stands where that app's test does, outside `:core`, so it compiles only
 * while every row's exception can be made from there.
 */
class CoreExceptionsTest {
    private val made: List<CraftException> =
        listOf(
            InvalidParamsException("width", WORDS),
            CommandFailedException("file.open", "missing `path`", 1u, WORDS),
            NotFoundException("comp", WORDS),
            UnsupportedException("export", WORDS),
            CancelledException(WORDS),
            IoException("disk", WORDS),
            GpuException("device", WORDS),
            InternalErrorException("panic", WORDS),
        )

    @Test
    fun anAppsOwnTestMakesEachOfTheCoresFailuresWithTheWordsItWants() {
        assertEquals(made.size, made.map { it::class }.distinct().size, "a row was made twice: $made")
        assertEquals(listOf(WORDS), made.map { it.message }.distinct(), "a failure did not keep the words it was made with")
    }

    @Test
    fun noConstructorDefaultsTheWordsBecauseThatWouldCopyTheCoresRuleForThem() {
        // Kotlin compiles a default into a synthetic constructor whose last two parameters are the mask of
        // the arguments left out and a marker; a constructor taking a `UInt` has a synthetic one too, with
        // the marker alone, which is no default.
        val defaulting =
            made.map { it::class.java }.filter { row ->
                row.declaredConstructors.any { constructor ->
                    val types = constructor.parameterTypes
                    constructor.isSynthetic && types.size >= 2 && types[types.size - 2] == Int::class.javaPrimitiveType
                }
            }
        assertEquals(emptyList(), defaulting, "a constructor has a default, so an app's test can leave the words out")
    }

    private companion object {
        const val WORDS = "the words an app's test wants to show"
    }
}
