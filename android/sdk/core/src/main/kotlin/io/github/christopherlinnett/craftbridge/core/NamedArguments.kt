// What makes every call of a generated command facade name its arguments.

package io.github.christopherlinnett.craftbridge.core

/**
 * The type of the `vararg` every generated command method takes first, which no call can fill: the type
 * has no value, so an argument passed by position, which would fill it, does not compile, and every
 * parameter after a `vararg` can only be named. Every argument is therefore passed by name, as the Swift
 * facade's labels pass every one of its own, and a key the engine adds before another can never take an
 * old call's value in its place. Kotlin has no named-only parameter of its own.
 */
public class NamedArguments private constructor()
