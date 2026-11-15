package io.github.christopherlinnett.craftbridge.convention

import com.android.build.api.variant.Component
import org.gradle.api.GradleException
import org.gradle.api.Project
import kotlin.io.path.invariantSeparatorsPathString

/**
 * Refuse Kotlin or Java that no compilation reads: every `.kt`, `.kts` and `.java` file under the
 * module's `src` must lie in a Kotlin or Java folder of one of the compilations AGP builds for the
 * module, as [Component.sources] lists them; the Kotlin compilation reads a script there as it reads
 * a `.kt`. Anywhere else a file is never compiled, though Spotless formats a `.kt` as written Kotlin,
 * so its type errors and warnings pass the gate unseen:
 * `src/testRelease/kotlin`, which AGP 9 compiles for nothing because a library variant has no release
 * test component, a misspelt `src/main/kotin`, or a `.kt` left among `src/main/resources`. This is
 * the Kotlin shape of what AGENTS §3 requires of Rust, that every source file belongs to a
 * compilation. Files are what is judged, not folders: a folder that holds none, which git cannot
 * keep, is no part of the module.
 *
 * The comparison runs when the task graph is ready, once AGP has listed every component. The walk
 * over `src` reads the file system at configuration time, which the configuration cache records as an
 * input of the build's configuration, so a tree that gains a file is judged again rather than answered
 * from the entry stored for the tree before it.
 */
internal fun Project.refuseUnreadSources() {
    val components = mutableListOf<Component>()
    forEachComponent(main = { components.add(it) }, test = { components.add(it) })
    val sources = fileTree("src") { include("**/*.kt", "**/*.kts", "**/*.java") }
    gradle.taskGraph.whenReady {
        val read =
            components
                .flatMap { component -> listOfNotNull(component.sources.kotlin?.all, component.sources.java?.all) }
                .flatMap { declared -> declared.get().map { it.asFile.canonicalFile.toPath() } }
                .toSet()
        val module = projectDir.toPath()
        // Each file is named by the source folder it sits in, `src/<source set>/<kind>`, or by the
        // folder it sits in when that is higher: one name for however many files a folder holds.
        val unread =
            sources.files
                .filter { file -> read.none { file.canonicalFile.toPath().startsWith(it) } }
                .map { file -> module.relativize(file.toPath()).let { it.subpath(0, minOf(3, it.nameCount - 1)) } }
                .map { it.invariantSeparatorsPathString }
                .distinct()
                .sorted()
        if (unread.isNotEmpty()) {
            throw GradleException(
                "$path keeps Kotlin or Java in ${unread.joinToString()}, which none of its compilations reads: every Kotlin and " +
                    "Java file of a module lies in a folder one of them compiles (AGENTS §3), and a file anywhere else is never compiled",
            )
        }
    }
}
