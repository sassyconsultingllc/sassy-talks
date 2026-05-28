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
# Rely on ML Kit's bundled consumer-rules ProGuard config (ships with the
# AAR). The previous blanket `-keep class com.google.mlkit.** { *; }` kept
# every internal class including ones the app never references, blocking
# R8 dead-code elimination. Only suppress the dontwarn — that's harmless.
-dontwarn com.google.mlkit.**

# --- Compose: keep only the runtime types R8 has trouble with on its own ---
# The previous blanket `-keep class androidx.compose.** { *; }` defeated
# ~30 % of R8's dead-code elimination because it kept every internal tooling
# class. Compose ships its own consumer-rules that handle @Composable
# preservation correctly; we only need to keep the runtime-state types
# referenced via reflection by Compose itself.
-keep class androidx.compose.runtime.snapshots.** { *; }
-keep class androidx.compose.runtime.MutableState { *; }
-keep class androidx.compose.runtime.SnapshotMutationPolicy { *; }
-dontwarn androidx.compose.**

# --- R8 full mode: keep enum values for when() exhaustiveness ---
-keepclassmembers enum * {
    public static **[] values();
    public static ** valueOf(java.lang.String);
}
