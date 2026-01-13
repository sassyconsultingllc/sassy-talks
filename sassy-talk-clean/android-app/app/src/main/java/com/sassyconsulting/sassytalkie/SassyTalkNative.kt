package com.sassyconsulting.sassytalkie

import android.util.Log

/**
 * JNI bridge to Rust native library
 */
object SassyTalkNative {
    
    private const val TAG = "SassyTalkNative"
    private var initialized = false
    
    init {
        try {
            System.loadLibrary("sassytalkie")
            Log.i(TAG, "Native library loaded successfully")
        } catch (e: UnsatisfiedLinkError) {
            Log.e(TAG, "Failed to load native library: ${e.message}")
        }
    }
    
    fun init(): Boolean {
        return try {
            initialized = nativeInit()
            Log.i(TAG, "Native init: $initialized")
            initialized
        } catch (e: Exception) {
            Log.e(TAG, "Init failed: ${e.message}")
            false
        }
    }
    
    fun pttStart() {
        if (initialized) {
            nativePttStart()
            Log.d(TAG, "PTT Started")
        }
    }
    
    fun pttStop() {
        if (initialized) {
            nativePttStop()
            Log.d(TAG, "PTT Stopped")
        }
    }
    
    fun setChannel(channel: Int) {
        if (initialized && channel in 1..99) {
            nativeSetChannel(channel.toByte())
            Log.d(TAG, "Channel set to $channel")
        }
    }
    
    // Native method declarations - these match the Rust FFI exports
    @JvmStatic
    private external fun nativeInit(): Boolean
    
    @JvmStatic
    private external fun nativePttStart()
    
    @JvmStatic
    private external fun nativePttStop()
    
    @JvmStatic
    private external fun nativeSetChannel(channel: Byte)
}
