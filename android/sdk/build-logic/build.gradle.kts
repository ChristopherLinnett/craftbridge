// build-logic's root project, which holds no code of its own. Its build-script classpath carries the
// one licence listing `:detekt-rules`' `gateToolLicences` prints, which every project of this build
// inherits from here. Declared in `:detekt-rules`' own `buildscript` block instead, the classpath gives
// that project a script classloader of its own, the Kotlin Gradle plugin is then loaded once for it and
// once for `:convention`, and the plugin warns that this is unsupported and may break the build.
buildscript {
    dependencies {
        classpath("io.github.christopherlinnett.craftbridge.build:licence-listing")
    }
}
