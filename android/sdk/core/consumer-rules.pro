# What an app that shrinks with R8 in full mode must keep for the generated bindings to reach the native
# library through JNA. Each rule is the narrowest that does its one job, and without any one of them an
# app's R8 build or its first call into the library fails; they cover the plugin's bindings too, which
# match by JNA's own types in any package.

# JNA names java.awt classes (its desktop window support) that Android does not have; nothing here calls
# them, and R8 fails the build on a class it cannot find. The rule names the two JNA classes that refer to
# them, which R8 matches as the referring side, rather than java.awt itself: an app takes these rules into
# its own, where silencing java.awt would hide every other library's missing reference to it too.
-dontwarn com.sun.jna.Native,com.sun.jna.Native$AWT

# JNA's native half looks its classes, fields and methods up by name through JNI (`Pointer.peer`, and the
# rest), so renaming or removing any of them breaks library loading.
-keep class com.sun.jna.* { *; }

# JNA reads each Structure's fields, and calls each Callback's one method, by reflection on the names the
# bindings declare, so those public members keep their names.
-keepclassmembers class * extends com.sun.jna.* { public *; }

# A Structure's layout is the order named by its annotation, which R8 drops from a class whose annotation
# it sees no code reading, though JNA reads it by reflection. Only the annotation is kept: JNA never asks
# a structure's class by name, and one no code reaches may still go.
-keep,allowobfuscation,allowshrinking @com.sun.jna.Structure$FieldOrder class *
