package com.sassyconsulting.sassytalkie

/**
 * JNI Bridge to Rust native library
 * 
 * This class provides the interface between Kotlin and the Rust backend.
 * The native library (libsassytalkie.so) handles:
 * - Audio encoding/decoding (Opus codec)
 * - Encryption (AES-256-GCM)
 * - Network transport (UDP multicast)
 * - Peer discovery
 */
object RustBridge {
    
    init {
        try {
            System.loadLibrary("sassytalkie")
        } catch (e: UnsatisfiedLinkError) {
            // Library not found - running without native code
            android.util.Log.w("RustBridge", "Native library not loaded: ${e.message}")
        }
    }
    
    // Audio control
    external fun startRecording(): Boolean
    external fun stopRecording()
    external fun startPlayback(): Boolean
    external fun stopPlayback()
    external fun setInputVolume(volume: Int)
    external fun setOutputVolume(volume: Int)
    
    // Network
    external fun startDiscovery(): Boolean
    external fun stopDiscovery()
    external fun connectToPeer(peerId: Long): Boolean
    external fun disconnectAll()
    
    // Channel management
    external fun setChannel(channel: Int)
    external fun getChannel(): Int
    
    // Peer info
    external fun getPeerCount(): Int
    external fun getPeerInfo(index: Int): String?
    
    // Security
    external fun runSecurityCheck(): Int
    external fun isEncryptionReady(): Boolean
    
    // Lifecycle
    external fun initialize(deviceName: String): Boolean
    external fun shutdown()
    
    /**
     * Fallback implementations when native library isn't loaded
     */
    @JvmStatic
    fun safeStartRecording(): Boolean {
        return try {
            startRecording()
        } catch (e: UnsatisfiedLinkError) {
            false
        }
    }
    
    @JvmStatic
    fun safeStopRecording() {
        try {
            stopRecording()
        } catch (e: UnsatisfiedLinkError) {
            // Ignore
        }
    }
    
    @JvmStatic
    fun safeInitialize(deviceName: String): Boolean {
        return try {
            initialize(deviceName)
        } catch (e: UnsatisfiedLinkError) {
            false
        }
    }
}
