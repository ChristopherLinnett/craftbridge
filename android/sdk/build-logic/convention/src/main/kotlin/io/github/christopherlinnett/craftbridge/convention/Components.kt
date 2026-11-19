package io.github.christopherlinnett.craftbridge.convention

import com.android.build.api.variant.AndroidComponentsExtension
import com.android.build.api.variant.ApplicationAndroidComponentsExtension
import com.android.build.api.variant.Component
import com.android.build.api.variant.LibraryAndroidComponentsExtension
import com.android.build.api.variant.Variant
import org.gradle.api.GradleException
import org.gradle.api.Project

/**
 * The module's own components extension, whichever kind of module it is: the variant API of its one
 * Android plugin, a library's or an app's. Both answer through [AndroidComponentsExtension] for what the
 * checks read (the SDK's paths, the variants' sources); only `onVariants` takes a type the base does not
 * name, so it is called on the kind itself.
 */
internal fun Project.androidComponents(): AndroidComponentsExtension<*, *, *> = extensions.getByType(AndroidComponentsExtension::class.java)

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
    val each: (Variant) -> Unit = { variant ->
        main(variant)
        variant.nestedComponents.forEach(test)
    }
    when (val components = androidComponents()) {
        is LibraryAndroidComponentsExtension -> components.onVariants { each(it) }
        is ApplicationAndroidComponentsExtension -> components.onVariants { each(it) }
        else -> throw GradleException("$path has neither the Android library nor the application plugin's variants to check")
    }
}
