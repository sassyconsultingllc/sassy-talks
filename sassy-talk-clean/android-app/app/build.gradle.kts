plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.sassyconsulting.sassytalkie"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.sassyconsulting.sassytalkie"
        minSdk = 24
        targetSdk = 35
        versionCode = 13
        versionName = "2.3.7"
        
        // Feature flag: enable or disable cellular (relay) transport at build time
        buildConfigField("boolean", "ENABLE_CELLULAR_RELAY", "true")
        buildConfigField("boolean", "NO_SCREENSHOTS", "false")

        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    signingConfigs {
        create("release") {
            // Signing credentials come exclusively from the environment so no
            // default password is ever baked into the build output. If the env
            // vars are unset, this signingConfig is simply not wired up for the
            // release buildType (we fall back to the debug keystore below).
            val ksFile = file("keystore/release.keystore")
            val envStorePw = System.getenv("RELEASE_STORE_PASSWORD")
            val envAlias = System.getenv("RELEASE_KEY_ALIAS")
            val envKeyPw = System.getenv("RELEASE_KEY_PASSWORD")
            if (ksFile.exists() &&
                !envStorePw.isNullOrBlank() &&
                !envAlias.isNullOrBlank() &&
                !envKeyPw.isNullOrBlank()
            ) {
                storeFile = ksFile
                storePassword = envStorePw
                keyAlias = envAlias
                keyPassword = envKeyPw
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
            val hasReleaseCreds = file("keystore/release.keystore").exists() &&
                !System.getenv("RELEASE_STORE_PASSWORD").isNullOrBlank() &&
                !System.getenv("RELEASE_KEY_ALIAS").isNullOrBlank() &&
                !System.getenv("RELEASE_KEY_PASSWORD").isNullOrBlank()
            // Allow local devs to opt into a debug-signed release build with
            // ALLOW_DEBUG_SIGNED_RELEASE=1, but never silently — and never on CI.
            val allowDebugSigned = System.getenv("ALLOW_DEBUG_SIGNED_RELEASE") == "1"
            val onCi = System.getenv("CI") == "true" || !System.getenv("GITHUB_ACTIONS").isNullOrBlank()
            signingConfig = when {
                hasReleaseCreds -> signingConfigs.getByName("release")
                onCi -> error(
                    "Release build on CI without release signing credentials. " +
                    "Set RELEASE_KEYSTORE_BASE64 / RELEASE_STORE_PASSWORD / " +
                    "RELEASE_KEY_ALIAS / RELEASE_KEY_PASSWORD secrets and re-run."
                )
                allowDebugSigned -> {
                    logger.warn("ALLOW_DEBUG_SIGNED_RELEASE=1 — signing release with the DEBUG keystore. NEVER upload this AAB.")
                    signingConfigs.getByName("debug")
                }
                else -> error(
                    "Release build without signing credentials. Set RELEASE_STORE_PASSWORD / " +
                    "RELEASE_KEY_ALIAS / RELEASE_KEY_PASSWORD env vars (and ensure " +
                    "app/keystore/release.keystore exists), or set ALLOW_DEBUG_SIGNED_RELEASE=1 " +
                    "for a throwaway debug-signed local build."
                )
            }
        }
    }
    
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    
    kotlinOptions {
        jvmTarget = "17"
    }
    
    buildFeatures {
        compose = true
        // Enable generation of BuildConfig fields used for feature flags
        buildConfig = true
    }
    
    composeOptions {
        kotlinCompilerExtensionVersion = "1.5.5"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.12.0")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.6.2")
    implementation("androidx.lifecycle:lifecycle-process:2.6.2")
    implementation("androidx.activity:activity-compose:1.8.1")
    
    // Compose
    implementation(platform("androidx.compose:compose-bom:2023.10.01"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")
    
    // Icons
    implementation("androidx.compose.material:material-icons-extended")

    // QR Code generation
    implementation("com.google.zxing:core:3.5.2")

    // QR Code scanning (ML Kit barcode)
    implementation("com.google.mlkit:barcode-scanning:17.2.0")

    // CameraX for QR scanner
    implementation("androidx.camera:camera-camera2:1.3.1")
    implementation("androidx.camera:camera-lifecycle:1.3.1")
    implementation("androidx.camera:camera-view:1.3.1")

    // JSON parsing
    implementation("org.json:json:20231013")

    // OkHttp for WebSocket (cellular relay transport)
    implementation("com.squareup.okhttp3:okhttp:4.12.0")

    // Android Keystore-backed EncryptedSharedPreferences for session/key storage.
    // Plain SharedPreferences is sandboxed to our UID but is cleartext on disk —
    // any backup or rooted access would leak the session key.
    implementation("androidx.security:security-crypto:1.1.0-alpha06")
    
    // AppCompat + Material + ConstraintLayout for legacy XML layouts
    implementation("androidx.appcompat:appcompat:1.6.1")
    implementation("com.google.android.material:material:1.9.0")
    implementation("androidx.constraintlayout:constraintlayout:2.1.4")
    
    debugImplementation("androidx.compose.ui:ui-tooling")

    testImplementation("junit:junit:4.13.2")
}

