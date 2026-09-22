// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-DFPOBSB4Y46O
package com.sassyconsulting.sassytalkie.audio

import kotlin.math.log10
import kotlin.math.sqrt

/**
 * Adaptive microphone gate.
 *
 * Modes:
 *  - Off          pass every frame (no gating)
 *  - Manual(thr)  fixed dBFS threshold
 *  - Auto(cfg)    calibrated noise-floor-relative threshold with hysteresis + hold
 *
 * Hysteresis: open threshold > close threshold prevents chatter on signals
 * hovering near the floor. Hold time keeps the gate open for N ms after the
 * last frame above the close threshold so word endings and breath pauses are
 * not chopped.
 *
 * NOT thread-safe; one gate per capture pipeline.
 */
class MicGate(
    val sampleRateHz: Int = 16_000,
    val frameSizeSamples: Int = 320, // 20 ms @ 16 kHz
) {
    sealed class Mode {
        object Off : Mode()
        data class Manual(val thresholdDbfs: Float) : Mode()
        data class Auto(val config: CalibratedConfig) : Mode()
    }

    data class CalibratedConfig(
        val noiseFloorDbfs: Float,
        val openThresholdDbfs: Float,
        val closeThresholdDbfs: Float,
        val holdMs: Int = 250,
        val calibratedAt: Long = System.currentTimeMillis(),
    )

    data class GateDecision(val transmit: Boolean, val frameDbfs: Float)

    @Volatile var mode: Mode = Mode.Off

    private var gateOpen = false
    private var holdUntilNanos = 0L
    private val nanosPerMs = 1_000_000L

    fun process(frame: ShortArray): GateDecision {
        val dbfs = computeDbfs(frame)
        val transmit = when (val m = mode) {
            is Mode.Off -> true
            is Mode.Manual -> dbfs >= m.thresholdDbfs
            is Mode.Auto -> applyHysteresis(dbfs, m.config)
        }
        return GateDecision(transmit, dbfs)
    }

    private fun applyHysteresis(dbfs: Float, cfg: CalibratedConfig): Boolean {
        val now = System.nanoTime()
        if (!gateOpen) {
            if (dbfs >= cfg.openThresholdDbfs) {
                gateOpen = true
                holdUntilNanos = now + cfg.holdMs * nanosPerMs
            }
        } else {
            if (dbfs >= cfg.closeThresholdDbfs) {
                holdUntilNanos = now + cfg.holdMs * nanosPerMs
            } else if (now > holdUntilNanos) {
                gateOpen = false
            }
        }
        return gateOpen
    }

    companion object {
        fun computeDbfs(frame: ShortArray): Float {
            if (frame.isEmpty()) return -120f
            var sumSq = 0.0
            for (s in frame) {
                val v = s.toDouble() / 32768.0
                sumSq += v * v
            }
            val rms = sqrt(sumSq / frame.size)
            if (rms <= 1e-9) return -120f
            return (20.0 * log10(rms)).toFloat()
        }
    }
}

fun MicGate.Mode.label(): String = when (this) {
    MicGate.Mode.Off -> "Off"
    is MicGate.Mode.Manual -> "Manual"
    is MicGate.Mode.Auto -> "Auto"
}

fun MicGate.Mode.threshold(): Float? = when (this) {
    MicGate.Mode.Off -> null
    is MicGate.Mode.Manual -> thresholdDbfs
    is MicGate.Mode.Auto -> config.openThresholdDbfs
}
