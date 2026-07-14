// Top-level build file
plugins {
    id("com.android.application") version "9.0.1" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.2.10" apply false
    // Firebase Cloud Messaging — only activated by the app module when
    // google-services.json is present. The plugin no-ops on its own.
    id("com.google.gms.google-services") version "4.4.2" apply false
}
