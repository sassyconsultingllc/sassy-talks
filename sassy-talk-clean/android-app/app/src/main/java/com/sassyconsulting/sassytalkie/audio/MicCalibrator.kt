package com.sassyconsulting.sassytalkie.audio

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

/**
 * Calibrates the MicGate by sampling ambient background frames and computing
 * a robust noise floor.
 *
 * Robust statistic: we use the 95th percentile of frame dBFS over the window
 * rather than the mean. This makes calibration immune to coughs, paper rustle,
 * keyboard taps, and other transients that would otherwise inflate the floor.
 *
 * Recommended margins:
 *   openMarginDb  = 12 dB above floor   (signal must be ~4x louder to trip)
 *   closeMarginDb =  6 dB above floor   (gate stays open while still ~2x floor)
 *
 * Quiet talkers and far-mic users: drop openMarginDb to 8 and closeMarginDb to 4.
 * Loud environments: raise to 15/9.
 */
class MicCalibrator(
    private val sampleRateHz: Int = 16_000,
    private val frameSizeSamples: Int = 320,
) {
    sealed class Event {
        data class Progress(
            val elapsedMs: Long,
            val totalMs: Long,
            val currentFrameDbfs: Float,
            val runningFloorDbfs: Float,
            val sampleCount: Int,
        ) : Event()

        data class Done(val config: MicGate.CalibratedConfig) : Event()

        data class Aborted(val reason: String) : Event()
    }

    /**
     * Consume a flow of ShortArray frames for [durationMs] milliseconds, then
     * emit a CalibratedConfig.
     *
     * The caller is responsible for sourcing frames from AudioRecord at the
     * same sampleRate/frameSize used here, and for stopping the input flow.
     */
    fun calibrate(
        frames: Flow<ShortArray>,
        durationMs: Long = 3000L,
        openMarginDb: Float = 12f,
        closeMarginDb: Float = 6f,
        holdMs: Int = 250,
        minSamples: Int = 50,
    ): Flow<Event> = flow {
        val startNanos = System.nanoTime()
        val readings = ArrayList<Float>(durationMs.toInt() / 20 + 16)

        frames.collect { frame ->
            val elapsedMs = (System.nanoTime() - startNanos) / 1_000_000L
            val dbfs = MicGate.computeDbfs(frame)
            readings.add(dbfs)
            val floor = percentile(readings, 0.95f)

            emit(Event.Progress(elapsedMs, durationMs, dbfs, floor, readings.size))

            if (elapsedMs >= durationMs) {
                if (readings.size < minSamples) {
                    emit(Event.Aborted("Too few samples (${readings.size}); check mic capture"))
                } else {
                    val cfg = MicGate.CalibratedConfig(
                        noiseFloorDbfs = floor,
                        openThresholdDbfs = floor + openMarginDb,
                        closeThresholdDbfs = floor + closeMarginDb,
                        holdMs = holdMs,
                    )
                    emit(Event.Done(cfg))
                }
                return@collect
            }
        }
    }

    private fun percentile(values: List<Float>, p: Float): Float {
        if (values.isEmpty()) return -60f
        val sorted = values.sorted()
        val idx = ((sorted.size - 1) * p).toInt().coerceIn(0, sorted.lastIndex)
        return sorted[idx]
    }
}
