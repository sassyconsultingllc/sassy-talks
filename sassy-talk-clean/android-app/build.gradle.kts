// Top-level build file
plugins {
    id("com.android.application") version "8.7.3" apply false
    id("org.jetbrains.kotlin.android") version "1.9.20" apply false
    // Firebase Cloud Messaging — only activated by the app module when
    // google-services.json is present. The plugin no-ops on its own.
    id("com.google.gms.google-services") version "4.4.2" apply false
}
