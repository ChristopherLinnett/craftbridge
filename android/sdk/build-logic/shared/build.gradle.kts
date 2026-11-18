// The code both sides of build-logic load, each piece defined once: the licence listing both Gradle
// sides print — `runtimeLicences` (from the convention plugin, over the library modules' classpaths)
// and `gateToolLicences` (from the rule project's own build file, over the gate tooling's) print what
// `LicenceListing` collects — and the written-against-run test count, which `craftbridge.android.library`
// wires onto every module's unit tests and `:detekt-rules` onto its own. This is a build of its own,
// under build-logic's own folder, because neither caller can hold the shared code: a build script
// cannot load a project of its own build onto its classpath — Gradle answers a project dependency in
// a `buildscript` block with "Project dependencies cannot be declared here" — and each of the two
// builds reaches the other only through coordinates, so a third build is the only code both can load.
// It has no settings file: Gradle treats a folder without one as a single-project build, so the gate's
// two allowed settings files (AGENTS §10) stay two.

plugins {
    `embedded-kotlin`
}

group = "io.github.christopherlinnett.craftbridge.build"

// An included build shares no configuration with the build that includes it, repositories included,
// and the implicit settings of a folder without one declare none; the stdlib and reflect that the
// embedded Kotlin resolves come from Maven Central, the one place every coordinate of this build's
// Kotlin lives.
repositories {
    mavenCentral()
}

kotlin {
    // This code runs inside the Gradle daemon — the licence listing prints there, and the count's
    // verifier task runs there — so it compiles against the JDK Gradle itself runs on: the same fact
    // the catalog's `jdk` records for the other build-logic projects, read here because a build of
    // its own shares nothing, catalogs included.
    compilerOptions {
        jvmTarget =
            org.jetbrains.kotlin.gradle.dsl.JvmTarget
                .fromTarget(JavaVersion.current().majorVersion)
        // Gate machinery like the rest of build-logic: a warning here fails its build as one does there.
        allWarningsAsErrors.set(true)
    }
}

dependencies {
    // Compiled against Gradle's API; at run time Gradle's own classloader, which already holds it,
    // is the parent of every loader that loads this jar. TestNG is no dependency: the count reads
    // TestNG's own report file as the plain XML it is and names TestNG's own reporter as a string,
    // so nothing here needs TestNG's classes to compile or to run.
    compileOnly(gradleApi())
}
