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
