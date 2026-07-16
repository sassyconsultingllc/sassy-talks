// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-4ZLUNKBYYT3T
package com.sassyconsulting.sassytalkie.audio

import android.util.Log

/**
 * Kotlin wrapper around libopus. PTT-optimized defaults are applied on init:
 *
 *   bitrate         = 24 kbps   (clean voice; raise to 32k for high quality)
 *   complexity      = 10        (modern phones can take it)
 *   application     = VOIP      (low-latency mode)
 *   signal          = VOICE     (better low-bitrate intelligibility)
 *   vbr             = on
 *   dtx             = OFF       (never let the codec gate PTT speech)
 *   inband FEC      = on        (lost packets reconstruct from neighbors)
 *   loss percent    = 10%       (tune FEC for expected loss rate)
 *
 * Frame size: 320 samples / 20 ms @ 16 kHz. Opus also supports 2.5/5/10/40/60 ms
 * but 20 ms is the standard for VoIP and what the rest of the pipeline assumes.
 */
class OpusEncoder(
    val sampleRateHz: Int = 16_000,
    val channels: Int = 1,
    val frameSizeSamples: Int = 320,
) : AutoCloseable {

    init {
        check(nativeAvailable) {
            "libsassytalkie_opus.so not loaded — drop libopus.so prebuilts under " +
                "src/main/jniLibs/<abi>/ and rebuild. See src/main/cpp/CMakeLists.txt."
        }
    }

    private var handle: Long = nativeCreate(sampleRateHz, channels)

    init {
        require(handle != 0L) {
            "opus_encoder_create failed — most likely cause: CMakeLists built the " +
                "stub library because src/main/cpp/include/opus/ and " +
                "src/main/jniLibs/<abi>/libopus.so are not vendored. Drop them in " +
                "and rebuild."
        }
        applyPttDefaults()
    }

    fun applyPttDefaults() {
        setBitrate(24_000)
        setComplexity(10)
        setApplication(APPLICATION_VOIP)
        setSignal(SIGNAL_VOICE)
        setVbr(true)
        setDtx(false)
        setInbandFec(true)
        setPacketLossPerc(10)
    }

    fun encode(pcm: ShortArray): ByteArray {
        check(handle != 0L) { "encoder closed" }
        require(pcm.size == frameSizeSamples * channels) {
            "frame must be $frameSizeSamples samples * $channels ch, got ${pcm.size}"
        }
        return nativeEncode(handle, pcm, frameSizeSamples)
            ?: throw IllegalStateException("opus_encode returned null")
    }

    fun setBitrate(bps: Int) = nativeSetBitrate(handle, bps)
    fun setComplexity(c: Int) = nativeSetComplexity(handle, c.coerceIn(0, 10))
    fun setApplication(app: Int) = nativeSetApplication(handle, app)
    fun setSignal(sig: Int) = nativeSetSignal(handle, sig)
    fun setVbr(on: Boolean) = nativeSetVbr(handle, on)
    fun setDtx(on: Boolean) = nativeSetDtx(handle, on)
    fun setInbandFec(on: Boolean) = nativeSetInbandFec(handle, on)
    fun setPacketLossPerc(p: Int) = nativeSetPacketLossPerc(handle, p.coerceIn(0, 100))

    override fun close() {
        if (handle != 0L) {
            nativeDestroy(handle)
            handle = 0L
        }
    }

    private external fun nativeCreate(sr: Int, ch: Int): Long
    private external fun nativeDestroy(h: Long)
    private external fun nativeEncode(h: Long, pcm: ShortArray, frameSize: Int): ByteArray?
    private external fun nativeSetBitrate(h: Long, br: Int)
    private external fun nativeSetComplexity(h: Long, c: Int)
    private external fun nativeSetApplication(h: Long, app: Int)
    private external fun nativeSetSignal(h: Long, sig: Int)
    private external fun nativeSetVbr(h: Long, vbr: Boolean)
    private external fun nativeSetDtx(h: Long, dtx: Boolean)
    private external fun nativeSetInbandFec(h: Long, fec: Boolean)
    private external fun nativeSetPacketLossPerc(h: Long, p: Int)

    companion object {
        private const val TAG = "OpusEncoder"

        const val APPLICATION_VOIP = 2048
        const val APPLICATION_AUDIO = 2049
        const val APPLICATION_RESTRICTED_LOWDELAY = 2051

        const val SIGNAL_AUTO = -1000
        const val SIGNAL_VOICE = 3001
        const val SIGNAL_MUSIC = 3002

        /**
         * True once libsassytalkie_opus has loaded successfully. We don't crash
         * the whole app at class-init time if libopus prebuilts haven't been
         * dropped in yet — only the constructor refuses to run.
         */
        @JvmStatic
        val nativeAvailable: Boolean = run {
            try {
                System.loadLibrary("sassytalkie_opus")
                true
            } catch (t: Throwable) {
                Log.w(TAG, "libsassytalkie_opus not available: ${t.message}")
                false
            }
        }
    }
}
