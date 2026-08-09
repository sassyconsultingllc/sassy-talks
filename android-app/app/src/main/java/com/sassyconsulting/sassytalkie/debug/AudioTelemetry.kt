// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-5MLQ7SZ4N6YW
package com.sassyconsulting.sassytalkie.debug

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import java.util.concurrent.atomic.AtomicLong

/**
 * Live audio + network telemetry. Threadsafe singleton.
 *
 * Counters are rolling per-second; call tickPerSecond() from a 1Hz timer
 * (typically a coroutine in the foreground service).
 *
 * UI consumes state via collectAsState(); writes are constant-time so this
 * is safe to call from the capture thread.
 */
object AudioTelemetry {

    data class State(
        val currentFrameDbfs: Float = -120f,

        // Gate
        val gateMode: String = "Off",
        val gateThresholdDbfs: Float? = null,
        val gateOpen: Boolean = true,
        val gateNoiseFloorDbfs: Float? = null,

        // TX rates (per second, updated by tickPerSecond)
        val framesCapturedPerSec: Long = 0,
        val framesGatedPerSec: Long = 0,
        val txPacketsPerSec: Long = 0,
        val txKbpsAvg: Float = 0f,

        // RX rates
        val rxPacketsPerSec: Long = 0,
        val rxErrorsPerSec: Long = 0,
        val rxKbpsAvg: Float = 0f,

        // Packet sizing
        val lastTxPacketBytes: Int = 0,
        val lastRxPacketBytes: Int = 0,

        // AEAD outcomes for inbound frames (from native). Splitting these out
        // is what makes "arriving but not decrypting" visible — the failure
        // mode where every wire counter looks healthy and no audio is heard.
        val cryptoRxOk: Long = 0,
        val cryptoRxFail: Long = 0,
        val cryptoRxNoSession: Long = 0,
        val cryptoRxReplay: Long = 0,
        /** -1 until at least one frame has been attempted. */
        val cryptoRxOkPct: Float = -1f,

        // Effects state
        val aecActive: Boolean = false,
        val nsActive: Boolean = false,
        val agcActive: Boolean = false,
        val audioSource: String = "unknown",
        val outputCommMode: Boolean = false,

        // Network
        val networkPath: String = "unknown",  // "cloud" | "bt" | "offline"
        val wsState: String = "unknown",
        val rttMs: Int? = null,
        val lastHeartbeatAgoMs: Long? = null,

        // Session / relay (for audio troubleshooting)
        val relayRoom: String = "",
        val cellularState: String = "",
        val wsRelayConnected: Boolean = false,
        val cellularSent: Long = 0,
        val cellularReceived: Long = 0,
        val inboundQueue: Int = 0,
        val outboundQueue: Int = 0,
        val droppedPackets: Long = 0,
        val wsSendDrops: Long = 0,
        val jitterPrebufferFrames: Int = 0,
        val jitterAdaptiveExtra: Int = 0,
        val jitterEffectiveFrames: Int = 0,
        val jitterEwmaMs: Float = 0f,
        val activeChannel: Int = 0,
        val peerCount: Int = 0,
        val usersInRegistry: Int = 0,
        val roomMatch: Boolean = true,

        // Device
        val deviceLabel: String = "${android.os.Build.MANUFACTURER} ${android.os.Build.MODEL}",
        val quirkNotes: String = "",
    )

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private val framesCaptured = AtomicLong(0)
    private val framesGated = AtomicLong(0)
    private val txPackets = AtomicLong(0)
    private val txBytes = AtomicLong(0)
    private val rxPackets = AtomicLong(0)
    private val rxBytes = AtomicLong(0)
    private val rxErrors = AtomicLong(0)

    fun onFrameCaptured(dbfs: Float, transmitted: Boolean) {
        framesCaptured.incrementAndGet()
        if (!transmitted) framesGated.incrementAndGet()
        _state.update { it.copy(currentFrameDbfs = dbfs) }
    }

    fun onPacketSent(bytes: Int) {
        txPackets.incrementAndGet()
        txBytes.addAndGet(bytes.toLong())
        _state.update { it.copy(lastTxPacketBytes = bytes) }
    }

    fun onPacketReceived(bytes: Int) {
        rxPackets.incrementAndGet()
        rxBytes.addAndGet(bytes.toLong())
        _state.update { it.copy(lastRxPacketBytes = bytes) }
    }

    fun onDecodeError() { rxErrors.incrementAndGet() }

    /**
     * Drive the CAPTURE / CODEC sections from the native counters.
     *
     * These fields previously had exactly one producer — the Kotlin
     * `PttAudioPipeline` — which is never constructed anywhere in the app.
     * Capture, encoding and encryption all happen in Rust, so Kotlin never saw
     * a frame and the panel showed boot defaults forever. The values now come
     * from `SassyTalkNative.diagSnapshot()`, which counts them where the work
     * actually occurs.
     *
     * Call AFTER [tickPerSecond] in the same pass: tick rolls the (unfed)
     * Kotlin counters and would otherwise zero these straight back out.
     * Deltas are per-second because the caller polls at 1 Hz.
     */
    fun updateFromNative(
        dbfs: Float,
        capturedPerSec: Long,
        encodedPerSec: Long,
        encodedBytesPerSec: Long,
    ) = _state.update {
        it.copy(
            currentFrameDbfs = dbfs,
            framesCapturedPerSec = capturedPerSec,
            txPacketsPerSec = encodedPerSec,
            txKbpsAvg = encodedBytesPerSec * 8f / 1000f,
        )
    }

    /**
     * AEAD outcomes for inbound frames. `okPct` is -1 when nothing has been
     * attempted yet. A climbing [cryptoRxFail] with [cryptoRxOk] stuck at zero
     * is the session-key-mismatch signature: traffic is arriving and being
     * discarded, which no other counter distinguishes from silence.
     */
    fun updateCryptoRx(ok: Long, fail: Long, noSession: Long, replay: Long, okPct: Float) =
        _state.update {
            it.copy(
                cryptoRxOk = ok,
                cryptoRxFail = fail,
                cryptoRxNoSession = noSession,
                cryptoRxReplay = replay,
                cryptoRxOkPct = okPct,
            )
        }

    fun updateGate(
        mode: String,
        threshold: Float?,
        open: Boolean,
        noiseFloor: Float? = null,
    ) = _state.update {
        it.copy(
            gateMode = mode,
            gateThresholdDbfs = threshold,
            gateOpen = open,
            gateNoiseFloorDbfs = noiseFloor ?: it.gateNoiseFloorDbfs,
        )
    }

    fun updateEffects(
        aec: Boolean, ns: Boolean, agc: Boolean,
        audioSource: String, outputCommMode: Boolean,
    ) = _state.update {
        it.copy(
            aecActive = aec, nsActive = ns, agcActive = agc,
            audioSource = audioSource, outputCommMode = outputCommMode,
        )
    }

    fun updateNetwork(path: String, wsState: String, rttMs: Int?, hbAgoMs: Long?) =
        _state.update {
            it.copy(
                networkPath = path,
                wsState = wsState,
                rttMs = rttMs,
                lastHeartbeatAgoMs = hbAgoMs,
            )
        }

    /** Relay room, cellular queue stats, and peer roster — polled ~1 Hz from WalkieService. */
    fun updateRelay(
        relayRoom: String,
        cellularState: String,
        wsRelayConnected: Boolean,
        sent: Long,
        received: Long,
        inboundQ: Int,
        outboundQ: Int,
        dropped: Long,
        activeChannel: Int,
        peerCount: Int,
        usersInRegistry: Int,
        roomMatch: Boolean,
        wsSendDrops: Long = 0,
        jitterPrebufferFrames: Int = 0,
        jitterAdaptiveExtra: Int = 0,
        jitterEffectiveFrames: Int = 0,
        jitterEwmaMs: Float = 0f,
    ) = _state.update {
        it.copy(
            relayRoom = relayRoom,
            cellularState = cellularState,
            wsRelayConnected = wsRelayConnected,
            cellularSent = sent,
            cellularReceived = received,
            inboundQueue = inboundQ,
            outboundQueue = outboundQ,
            droppedPackets = dropped,
            activeChannel = activeChannel,
            peerCount = peerCount,
            usersInRegistry = usersInRegistry,
            roomMatch = roomMatch,
            wsSendDrops = wsSendDrops,
            jitterPrebufferFrames = jitterPrebufferFrames,
            jitterAdaptiveExtra = jitterAdaptiveExtra,
            jitterEffectiveFrames = jitterEffectiveFrames,
            jitterEwmaMs = jitterEwmaMs,
        )
    }

    fun updateQuirkNotes(notes: String) = _state.update { it.copy(quirkNotes = notes) }

    fun tickPerSecond() {
        val tp = txPackets.getAndSet(0)
        val tb = txBytes.getAndSet(0)
        val rp = rxPackets.getAndSet(0)
        val rb = rxBytes.getAndSet(0)
        val fc = framesCaptured.getAndSet(0)
        val fg = framesGated.getAndSet(0)
        val re = rxErrors.getAndSet(0)
        _state.update {
            it.copy(
                txPacketsPerSec = tp,
                txKbpsAvg = tb * 8f / 1000f,
                rxPacketsPerSec = rp,
                rxKbpsAvg = rb * 8f / 1000f,
                framesCapturedPerSec = fc,
                framesGatedPerSec = fg,
                rxErrorsPerSec = re,
            )
        }
    }
}
