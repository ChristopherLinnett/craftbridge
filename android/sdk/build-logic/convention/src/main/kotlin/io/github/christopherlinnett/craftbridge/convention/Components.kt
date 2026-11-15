package io.github.christopherlinnett.craftbridge.convention

import com.android.build.api.variant.Component
import com.android.build.api.variant.LibraryAndroidComponentsExtension
import org.gradle.api.Project
import org.gradle.kotlin.dsl.getByType

/**
 * The compilations AGP builds for the module, read once through AGP's own variant API: [main]
 * gets each variant's own component, [test] every component that tests a variant — its unit-test
 * and instrumented-test compilations. A build type or a flavor added later brings its components
 * with it; nothing here names one. detekt's tasks and the refusal of unread sources read through
 * this one walk, so they see the same compilations and AGP's variant list is taken from one place.
 */
internal fun Project.forEachComponent(
    main: (Component) -> Unit,
    test: (Component) -> Unit,
) {
    extensions
        .getByType<LibraryAndroidComponentsExtension>()
        .onVariants { variant ->
            main(variant)
            variant.nestedComponents.forEach(test)
        }
}
