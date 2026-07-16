// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-P4CKB4VFI3SE
// Top-level build file
plugins {
    id("com.android.application") version "9.0.1" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.2.10" apply false
    // Firebase Cloud Messaging — only activated by the app module when
    // google-services.json is present. The plugin no-ops on its own.
    id("com.google.gms.google-services") version "4.4.2" apply false
}
