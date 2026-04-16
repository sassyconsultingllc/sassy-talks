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

# --- Kotlin coroutines ---
-dontwarn kotlinx.coroutines.**
-keep class kotlinx.coroutines.** { *; }

# --- OkHttp ---
-dontwarn okhttp3.internal.platform.**
-dontwarn org.conscrypt.**
-dontwarn org.bouncycastle.**
-dontwarn org.openjsse.**

# --- ML Kit barcode scanning ---
-keep class com.google.mlkit.** { *; }
-dontwarn com.google.mlkit.**

# --- Compose: keep @Composable lambdas and state ---
-keep class androidx.compose.** { *; }
-dontwarn androidx.compose.**

# --- R8 full mode: keep enum values for when() exhaustiveness ---
-keepclassmembers enum * {
    public static **[] values();
    public static ** valueOf(java.lang.String);
}
