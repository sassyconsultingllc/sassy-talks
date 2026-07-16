# Keep native methods
-keepclasseswithmembernames class * {
    native <methods>;
}

# Keep SassyTalkNative class
-keep class com.sassyconsulting.sassytalkie.SassyTalkNative { *; }

# Keep TranscriptionBridge (called from Rust JNI)
-keep class com.sassyconsulting.sassytalkie.TranscriptionBridge { *; }

# Keep WalkieService (foreground service)
-keep class com.sassyconsulting.sassytalkie.WalkieService { *; }

# Keep CellularWebSocketClient (OkHttp WebSocket callbacks)
-keep class com.sassyconsulting.sassytalkie.CellularWebSocketClient { *; }

# --- Seamless Connection (heartbeat, liveness, presence, delivery) ---

# ControlFrame — opcodes referenced by byte value; PresenceState enum used in heartbeat payload
-keep class com.sassyconsulting.sassytalkie.ControlFrame { *; }
-keep enum com.sassyconsulting.sassytalkie.PresenceState { *; }

# SessionEpoch — volatile singleton accessed across threads
-keep class com.sassyconsulting.sassytalkie.SessionEpoch { *; }

# Capabilities — JSON serialization uses field names via JSONObject.put/getString
-keep class com.sassyconsulting.sassytalkie.Capabilities { *; }

# LivenessTracker + PeerHealth — referenced from multiple coroutine scopes
-keep class com.sassyconsulting.sassytalkie.LivenessTracker { *; }
-keep enum com.sassyconsulting.sassytalkie.PeerHealth { *; }

# DeliveryState — enum collected by Compose UI
-keep enum com.sassyconsulting.sassytalkie.DeliveryState { *; }

# AudioFrameV2 — encode/decode called from BluetoothTransport callbacks
-keep class com.sassyconsulting.sassytalkie.AudioFrameV2 { *; }
-keep class com.sassyconsulting.sassytalkie.AudioV2Decoded { *; }

# PresenceSensor — accesses Android system services and ProcessLifecycleOwner
-keep class com.sassyconsulting.sassytalkie.PresenceSensor { *; }

# BluetoothTransport — has JNI-called callbacks (audioFrameCallback, txFrameCallback, audioFrameV2Callback)
-keep class com.sassyconsulting.sassytalkie.service.BluetoothTransport {
    public *;
    void audioFrameCallback(...);
    void txFrameCallback(...);
    void audioFrameV2Callback(...);
}

# PttCoordinator — central coordinator with flows collected by Compose
-keep class com.sassyconsulting.sassytalkie.PttCoordinator { *; }

# FCM wake-push service — declared in manifest, kept here so R8 doesn't strip
# the dispatch path R8 can't trace through Firebase's reflection.
-keep class com.sassyconsulting.sassytalkie.SassyTalkFcmService { *; }
-keep class com.sassyconsulting.sassytalkie.PresenceClient { *; }
-keep class com.sassyconsulting.sassytalkie.SessionShareLink { *; }
-keep class com.sassyconsulting.sassytalkie.InstallId { *; }
-dontwarn com.google.firebase.**

# --- Kotlin coroutines ---
-dontwarn kotlinx.coroutines.**
-keepnames class kotlinx.coroutines.internal.MainDispatcherFactory {}
-keepnames class kotlinx.coroutines.CoroutineExceptionHandler {}
-keepclassmembernames class kotlinx.coroutines.** {
    volatile <fields>;
}

# --- OkHttp ---
-dontwarn okhttp3.internal.platform.**
-dontwarn org.conscrypt.**
-dontwarn org.bouncycastle.**
-dontwarn org.openjsse.**

# --- ML Kit barcode scanning ---
# Blanket-keep ML Kit internals. ML Kit loads native barcode-scanning models
# via reflection-driven class lookup (initOptions, MlKitContext) and TensorFlow
# Lite interpreter loading; aggressive stripping can break these paths at
# runtime in ways that don't show up at startup — only when the QR scanner
# screen actually tries to scan. We keep everything to preserve those paths.
-keep class com.google.mlkit.** { *; }
-keep class com.google.android.gms.internal.mlkit_** { *; }
-dontwarn com.google.mlkit.**

# --- Firebase (used by FCM wake-push) ---
# Firebase initializes itself via a ContentProvider injected at app start,
# uses reflection extensively for component discovery, and the messaging
# service class is looked up by name. Keep the lot — wake-push is a
# foundational walkie-talkie path; a runtime miss here means peers can't
# wake each other from Doze.
-keep class com.google.firebase.** { *; }
-keep class com.google.android.gms.** { *; }
-dontwarn com.google.firebase.**
-dontwarn com.google.android.gms.**

# --- Compose: blanket-keep the entire runtime + UI tree ---
# Compose uses reflection in several places R8 can't statically trace:
# accessibility delegate lookup, @Stable / @Immutable introspection,
# slot-table key derivation, recomposition source-location maps, and
# the LayoutInspector tooling hooks. Aggressive stripping passed our
# unit tests but produced unpredictable runtime breakage on real screens
# (Settings, QR scanner, etc.). Preserve the entire androidx.compose
# tree — APK is ~5 MB larger but correctness is guaranteed.
-keep class androidx.compose.** { *; }
-keepclassmembers class androidx.compose.** { *; }
-dontwarn androidx.compose.**

# --- CameraX (QR scanner backbone) ---
# CameraX's lifecycle binding + use-case selection uses reflection.
-keep class androidx.camera.** { *; }
-dontwarn androidx.camera.**

# --- AppCompat + Material + ConstraintLayout (legacy XML layouts) ---
# These ARE used by some XML layouts the QR scanner still mounts; their
# consumer-rules cover most of it but on R8 full mode we've seen edge
# cases where MaterialButton / inflation paths got stripped.
-keep class com.google.android.material.** { *; }
-keep class androidx.appcompat.** { *; }
-keep class androidx.constraintlayout.** { *; }
-dontwarn com.google.android.material.**

# --- AndroidX Security Crypto (EncryptedSharedPreferences) ---
# Master-key generation uses reflection through Android KeyStore.
-keep class androidx.security.crypto.** { *; }
-dontwarn androidx.security.crypto.**

# --- R8 full mode: keep enum values for when() exhaustiveness ---
-keepclassmembers enum * {
    public static **[] values();
    public static ** valueOf(java.lang.String);
}
