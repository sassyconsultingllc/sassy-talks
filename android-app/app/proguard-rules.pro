# R8 rules for Sassy Talkie.
#
# Keep this file narrow. Play Console scores DEX optimization, obfuscation and
# shrinking, and every broad `-keep class lib.** { *; }` turns R8 off for that
# library. AndroidX (Compose, CameraX, core, security-crypto), Firebase, Play
# Services, ML Kit, OkHttp and Billing all ship consumer rules inside their
# AARs that already keep what their own reflection needs. Manifest components
# (services, receivers, providers, activities) are kept by AAPT-generated rules.
#
# Only add a rule here for something R8 cannot see: JNI lookups from Rust/C++
# by name, or our own reflection.

# --- JNI: native methods (SassyTalkNative, audio.OpusEncoder, ...) ---
# Native symbols are bound by Java_<class>_<method>, so the class and method
# names must survive.
-keepclasseswithmembernames,includedescriptorclasses class * {
    native <methods>;
}

# SassyTalkNative: the Rust library exports 124 Java_..._SassyTalkNative_*
# entry points.
-keep class com.sassyconsulting.sassytalkie.SassyTalkNative { *; }

# TranscriptionBridge: Rust does FindClass on it by name and calls the static
# onAudioReceived / onUtteranceCommitted methods (android-native/src/
# jni_bridge.rs, audio_pipeline.rs).
-keep class com.sassyconsulting.sassytalkie.TranscriptionBridge { *; }

# Rust permissions.rs does FindClass("androidx/core/content/ContextCompat") and
# calls checkSelfPermission by name.
-keep class androidx.core.content.ContextCompat {
    public static int checkSelfPermission(android.content.Context, java.lang.String);
}

# --- Firebase/ML Kit component registrars ---
# ComponentDiscovery reads registrar class names from manifest <meta-data> and
# calls the no-arg constructor by reflection. Firebase's own AARs keep theirs;
# ML Kit (translate, barcode, vision-common, common) does not, and R8 strips
# the constructor -> NoSuchMethodException at startup and a dead QR scanner and
# translator. Verified on a minified 3.2.8 build, 2026-11-07.
-keep class * implements com.google.firebase.components.ComponentRegistrar {
    public <init>();
}

# --- Optional TLS providers OkHttp / FipsProvider probe for at runtime ---
-dontwarn org.conscrypt.**
-dontwarn org.bouncycastle.**
-dontwarn org.openjsse.**

# --- Compile-only annotations referenced by Tink (security-crypto) ---
-dontwarn com.google.errorprone.annotations.**
-dontwarn javax.annotation.**

# --- Readable crash stack traces in Play Console (with the uploaded mapping) ---
-keepattributes SourceFile,LineNumberTable
-renamesourcefileattribute SourceFile
