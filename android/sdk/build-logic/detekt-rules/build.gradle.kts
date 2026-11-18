// The build's own detekt rules: the Kotlin rules of AGENTS.md §3, §4, §9 and §10 that detekt's own rule
// sets do not carry, written with detekt's own rule API so the checks read Kotlin's syntax tree like
// every other detekt rule (plan D-28). Each module's detekt runs load the jar as a plugin from a tool
// classpath the convention plugin builds — tooling running the gate's checks, not a dependency of what
// any module ships.

// The one definition of the licence listing that `gateToolLicences` below prints, shared with
// `runtimeLicences`: code of the build `licence-listing`, on build-logic's root build-script classpath
// (../build.gradle.kts), because a build script cannot load a project of its own build.
import io.github.christopherlinnett.craftbridge.licences.LicenceListing

// Kotlin at the version Gradle itself embeds, as kotlin-dsl compiles the convention plugins, without the
// Gradle-plugin tooling kotlin-dsl adds: this project is a jar of detekt rules, not a Gradle plugin.
plugins {
    `embedded-kotlin`
}

group = "io.github.christopherlinnett.craftbridge.build"

kotlin {
    // The rules run inside the detekt command line, which the gate starts on the JDK Gradle itself runs
    // on (the catalog's `jdk`, which tools/env.sh finds), so they compile against that JDK, as the
    // convention plugins do.
    compilerOptions {
        jvmTarget =
            org.jetbrains.kotlin.gradle.dsl.JvmTarget
                .fromTarget(libs.versions.jdk.get())
        // The rules are the gate's own code: a warning in them fails their build as one in a module does.
        allWarningsAsErrors.set(true)
    }
}

dependencies {
    // compileOnly because detekt loads rules from its own process classpath, where the API and the
    // engine already sit; the jar must not drag a second copy of either along.
    compileOnly(libs.detekt.api)
    compileOnly(libs.detekt.core)
    // The rules carry their own tests, which the android stage runs (tools/ci.sh). They run on TestNG:
    // kotlin-test binds to JUnit 4 by default, EPL-1.0 is on no branch of the licence allowlist, and
    // these test classpaths are gate machinery the allowlist holds like any other dependency.
    testImplementation(libs.detekt.test)
    testImplementation(libs.detekt.core)
    testImplementation(libs.kotlin.test.testng)
    // The binding drags a TestNG of its own along transitively, so the TestNG is named beside it, as
    // `craftbridge.android.library` names it for every module: the catalog's, strict.
    testImplementation(libs.testng)
}

tasks.test {
    // The framework is named, not left to kotlin-test's default binding; useTestNG also points
    // kotlin-test's own variant selection at the TestNG binding, so no JUnit arrives.
    useTestNG()
    // The profiles say which rules run, and a test reads them: a change to one runs the tests again.
    inputs.dir("profiles")
}

// The gate's own tooling, resolved for the licence check: `cargo xtask rules` asks Gradle
// (`runtimeLicences` and this task together), and every external module on any of these classpaths
// needs its licence in tools/licences.toml, as every module a library resolves does — AGENTS §8
// allows nothing outside the allowlist, "what ships and what the tests run with" alike. The rules'
// four classpaths are what their build and their tests run on; detekt-cli is the tool the convention
// plugin forks per component, resolved here from the same catalog coordinate and the same
// repositories the gate's detached classpath resolves, so the answer is the tree the gate runs.
val detektCliRuntime: org.gradle.api.artifacts.Configuration =
    configurations.detachedConfiguration(dependencies.create(libs.detekt.cli.get()))
val gateToolClasspaths: List<org.gradle.api.artifacts.Configuration> =
    listOf(
        configurations.getByName("compileClasspath"),
        configurations.getByName("runtimeClasspath"),
        configurations.getByName("testCompileClasspath"),
        configurations.getByName("testRuntimeClasspath"),
        detektCliRuntime,
    )
tasks.register("gateToolLicences") {
    description =
        "Print every external module resolved onto the gate's detekt tooling, one group:name per line."
    group = "verification"
    // The live dependency graph is read when the task runs and is not a serialisable task input,
    // exactly as it is for Gradle's own `dependencies` report task.
    notCompatibleWithConfigurationCache("reads the resolved dependency graph at execution time")
    doLast {
        val modules = sortedSetOf<String>()
        LicenceListing.addResolvedModules(gateToolClasspaths, modules)
        modules.forEach { println(it) }
    }
}
