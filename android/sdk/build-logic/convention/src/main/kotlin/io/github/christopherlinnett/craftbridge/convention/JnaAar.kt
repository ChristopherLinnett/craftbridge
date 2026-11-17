package io.github.christopherlinnett.craftbridge.convention

import org.gradle.api.artifacts.CacheableRule
import org.gradle.api.artifacts.ComponentMetadataContext
import org.gradle.api.artifacts.ComponentMetadataRule

/**
 * Hands out JNA's AAR instead of its jar. The module's POM says `jar`, and Android's packaging takes no
 * native library from a jar; the AAR published beside it carries `libjnidispatch.so` in each ABI's
 * folder of `jni/`, which an app packages where Android loads libraries from. It carries no R8 rules,
 * which is why `:core`'s consumer rules keep what JNA needs. A consumer cannot ask for it by coordinate
 * alone here, since a module's build file holds a catalog alias and no `@aar` (`cargo xtask rules`), so
 * the rule does what that notation would: it swaps the one file of each variant.
 */
@CacheableRule
abstract class JnaAar : ComponentMetadataRule {
    override fun execute(context: ComponentMetadataContext) {
        val id = context.details.id
        context.details.allVariants {
            withFiles {
                removeAllFiles()
                addFile("${id.name}-${id.version}.aar")
            }
        }
    }
}
