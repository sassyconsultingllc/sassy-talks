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

# OkHttp
-dontwarn okhttp3.internal.platform.**
-dontwarn org.conscrypt.**
-dontwarn org.bouncycastle.**
-dontwarn org.openjsse.**
