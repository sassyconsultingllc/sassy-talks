// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-VL3YQZI7AF2I
package com.sassyconsulting.sassytalkie.audio

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import androidx.annotation.RequiresPermission
import com.sassyconsulting.sassytalkie.debug.AudioTelemetry
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlin.math.max

/**
 * End-to-end PTT capture pipeline.
 *
 * Lifecycle:
 *   1. construct
 *   2. startCalibration() -> collect CalibrationEvent.Done(cfg), apply via setGateAuto(cfg)
 *   3. startTransmit() while PTT held -> packets emitted on `packets`
 *   4. stopTransmit() on PTT release
 *   5. shutdown() on session end
 *
 * Audio source selection: tries DeviceQuirks.current().sourceFallbackChain in
 * order until one produces an initialized AudioRecord. Reports the chosen
 * source in telemetry.
 *
 * Threading: capture runs on Dispatchers.Default. Frames are processed inline
 * (gate -> encode) to keep allocation-free fast path. Packets emit through a
 * SharedFlow with extraBufferCapacity to avoid blocking the audio thread.
 */
class PttAudioPipeline(
    context: Context,
    val sampleRateHz: Int = 16_000,
    val frameSizeSamples: Int = 320,
) {
    data class EncodedPacket(
        val opus: ByteArray,
        val captureNanos: Long,
        val frameDbfs: Float,
    )

    val gate = MicGate(sampleRateHz, frameSizeSamples)
    private val quirks = DeviceQuirks.current()
    private val router = AudioOutputRouter(context)

    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())
    private val _packets = MutableSharedFlow<EncodedPacket>(extraBufferCapacity = 64)
    val packets: Flow<EncodedPacket> = _packets.asSharedFlow()

    private var captureJob: Job? = null
    private var telemetryTickJob: Job? = null
    private var audioRecord: AudioRecord? = null
    private var effects: AudioEffectsManager? = null
    private var encoder: OpusEncoder? = null
    private var activeSourceName: String = "none"

    init {
        AudioTelemetry.updateQuirkNotes(quirks.notes)
        telemetryTickJob = scope.launch {
            while (isActive) {
                AudioTelemetry.tickPerSecond()
                delay(1000)
            }
        }
    }

    fun setGateMode(mode: MicGate.Mode) {
        gate.mode = mode
        AudioTelemetry.updateGate(
            mode = mode.label(),
            threshold = mode.threshold(),
            open = false,
            noiseFloor = (mode as? MicGate.Mode.Auto)?.config?.noiseFloorDbfs,
        )
    }

    /**
     * Open a capture session for calibration only. Yields raw frames; caller
     * passes them to MicCalibrator. Pipeline does NOT encode or emit packets.
     */
    @RequiresPermission(Manifest.permission.RECORD_AUDIO)
    fun calibrationFrames(): Flow<ShortArray> = callbackFlow {
        val record = openAudioRecord()
            ?: throw IllegalStateException("AudioRecord init failed for calibration")
        record.startRecording()
        val job = scope.launch {
            // Reuse one capture buffer; only copy when the collector needs a
            // stable snapshot (partial reads or overlapping consumption).
            val buf = ShortArray(frameSizeSamples)
            while (isActive) {
                val n = record.read(buf, 0, frameSizeSamples)
                if (n <= 0) continue
                // callbackFlow trySend is synchronous to the collector; MicCalibrator
                // processes immediately, so a shared full-frame buffer is safe.
                // Partial reads still need a sized copy.
                val frame = if (n == frameSizeSamples) buf else buf.copyOf(n)
                trySend(frame)
            }
        }
        awaitClose {
            job.cancel()
            try { record.stop() } catch (_: Throwable) {}
            record.release()
        }
    }

    @RequiresPermission(Manifest.permission.RECORD_AUDIO)
    fun startTransmit() {
        stopTransmit()
        val record = openAudioRecord()
            ?: error("Could not initialize any AudioRecord source from $activeSourceName")
        audioRecord = record

        effects = AudioEffectsManager(record.audioSessionId).also { mgr ->
            val applied = mgr.apply(quirks.effectsConfig)
            AudioTelemetry.updateEffects(
                aec = applied.aecActive,
                ns = applied.nsActive,
                agc = applied.agcActive,
                audioSource = activeSourceName,
                outputCommMode = router.isActive,
            )
        }

        encoder = OpusEncoder(sampleRateHz, 1, frameSizeSamples)
        record.startRecording()

        captureJob = scope.launch {
            val buf = ShortArray(frameSizeSamples)
            while (isActive) {
                val n = record.read(buf, 0, frameSizeSamples)
                if (n <= 0) continue
                val frame = if (n == frameSizeSamples) buf else buf.copyOf(frameSizeSamples)
                val decision = gate.process(frame)

                AudioTelemetry.onFrameCaptured(decision.frameDbfs, decision.transmit)
                AudioTelemetry.updateGate(
                    mode = gate.mode.label(),
                    threshold = gate.mode.threshold(),
                    open = decision.transmit,
                    noiseFloor = (gate.mode as? MicGate.Mode.Auto)?.config?.noiseFloorDbfs,
                )

                if (decision.transmit) {
                    val packet = try { encoder?.encode(frame) } catch (_: Throwable) { null }
                        ?: continue
                    AudioTelemetry.onPacketSent(packet.size)
                    _packets.tryEmit(
                        EncodedPacket(packet, System.nanoTime(), decision.frameDbfs)
                    )
                }
            }
        }
    }

    fun stopTransmit() {
        captureJob?.cancel(); captureJob = null
        try { audioRecord?.stop() } catch (_: Throwable) {}
        audioRecord?.release(); audioRecord = null
        effects?.release(); effects = null
        encoder?.close(); encoder = null
    }

    fun engageReceive() {
        router.engageCommMode(forceSpeaker = true)
        AudioTelemetry.updateEffects(
            aec = false, ns = false, agc = false,
            audioSource = activeSourceName,
            outputCommMode = router.isActive,
        )
    }

    fun releaseReceive() {
        router.release()
    }

    fun shutdown() {
        stopTransmit()
        releaseReceive()
        telemetryTickJob?.cancel()
        scope.cancel()
    }

    @SuppressLint("MissingPermission")
    private fun openAudioRecord(): AudioRecord? {
        val minBuf = AudioRecord.getMinBufferSize(
            sampleRateHz,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
        )
        if (minBuf <= 0) return null
        val bufBytes = max(
            minBuf * quirks.recordBufferMultiplier,
            frameSizeSamples * 2 * 8,
        )

        val chain = buildList {
            quirks.preferredAudioSource?.let { add(it) }
            addAll(quirks.sourceFallbackChain.filter { it != quirks.preferredAudioSource })
        }

        for (src in chain) {
            try {
                val rec = AudioRecord(
                    src,
                    sampleRateHz,
                    AudioFormat.CHANNEL_IN_MONO,
                    AudioFormat.ENCODING_PCM_16BIT,
                    bufBytes,
                )
                if (rec.state == AudioRecord.STATE_INITIALIZED) {
                    activeSourceName = sourceName(src)
                    return rec
                }
                rec.release()
            } catch (_: Throwable) { /* try next source */ }
        }
        return null
    }

    private fun sourceName(src: Int): String = when (src) {
        MediaRecorder.AudioSource.VOICE_RECOGNITION -> "VOICE_RECOGNITION"
        MediaRecorder.AudioSource.MIC -> "MIC"
        MediaRecorder.AudioSource.DEFAULT -> "DEFAULT"
        MediaRecorder.AudioSource.VOICE_COMMUNICATION -> "VOICE_COMMUNICATION"
        else -> "src=$src"
    }
}
