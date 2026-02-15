package com.sassyconsulting.sassytalkie

import android.util.Log

/**
 * JNI bridge to Rust native library.
 *
 * The native library (libsassytalkie.so) handles:
 * - Bluetooth RFCOMM + WiFi multicast transport
 * - AES-256-GCM encryption
 * - Audio capture and playback
 * - Smart transport selection (BT default, WiFi preferred)
 */
object SassyTalkNative {

    private const val TAG = "SassyTalkNative"
    private var initialized = false

    /** Transport type constants matching Rust enum */
    const val TRANSPORT_NONE = 0
    const val TRANSPORT_BLUETOOTH = 1
    const val TRANSPORT_WIFI = 2

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

    /** Get active transport: 0=None, 1=Bluetooth, 2=WiFi */
    fun getTransport(): Int {
        return if (initialized) {
            try {
                nativeGetTransport().toInt()
            } catch (e: Exception) {
                TRANSPORT_NONE
            }
        } else {
            TRANSPORT_NONE
        }
    }

    fun getTransportName(): String {
        return when (getTransport()) {
            TRANSPORT_BLUETOOTH -> "BT"
            TRANSPORT_WIFI -> "WiFi"
            else -> "---"
        }
    }

    // Native method declarations matching Rust FFI exports
    @JvmStatic private external fun nativeInit(): Boolean
    @JvmStatic private external fun nativePttStart()
    @JvmStatic private external fun nativePttStop()
    @JvmStatic private external fun nativeSetChannel(channel: Byte)
    @JvmStatic private external fun nativeGetTransport(): Byte
}
