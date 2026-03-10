package com.sassyconsulting.sassytalkie

import android.util.Log
import kotlinx.coroutines.*
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import com.sassyconsulting.sassytalkie.service.BluetoothTransport

/**
 * PttCoordinator — Orchestrates BLE signaling + RFCOMM data for PTT.
 *
 * TX Flow (sender presses PTT):
 * 1. BLE: broadcastPttStart() to all peers (instant, 1 byte)
 * 2. Wait for READY_ACK from peers (200ms timeout)
 * 3. Native: start mic capture + ADPCM encode
 * 4. RFCOMM: TX pump reads encoded frames -> sends to peers
 * 5. On release: stop TX, BLE broadcastPttStop()
 *
 * RX Flow (receiver gets BLE signal):
 * 1. BleSignalingService.onPttStartReceived()
 * 2. Send READY_ACK via BLE
 * 3. RFCOMM RX is already running (started on connect)
 * 4. Audio will arrive and be decoded by the RX thread
 *
 * Cache-first RX (audio -> ring buffer -> drain thread -> AudioTrack)
 *
 * BLE PTT_STOP -> play roger beep
 */
class PttCoordinator(
    private val bleSignaling: BleSignalingService,
    private val btTransport: BluetoothTransport
) : BleSignalingService.Listener {

    companion object {
        private const val TAG = "PTT.Coord"
        private const val READY_ACK_TIMEOUT_MS = 200L
    }

    private val transmitting = AtomicBoolean(false)
    private val readyAckCount = AtomicInteger(0)
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())

    init {
        bleSignaling.listener = this
    }

    // —— TX Side (We press PTT) ——

    fun onPttPressed() {
        if (transmitting.getAndSet(true)) return

        val blePeers = bleSignaling.blePeerCount
        val rfcommPeers = btTransport.connectedPeerCount

        Log.i(TAG, "PTT PRESSED \u2014 BLE peers: $blePeers, RFCOMM peers: $rfcommPeers")

        if (blePeers == 0 && rfcommPeers == 0) {
            Log.w(TAG, "PTT BLOCKED: No peers connected")
            transmitting.set(false)
            return
        }

        // Step 1: BLE signal to all peers
        readyAckCount.set(0)
        bleSignaling.broadcastPttStart()

        // Step 2: Brief wait for ACKs, then start audio regardless
        scope.launch {
            delay(READY_ACK_TIMEOUT_MS)
            val acks = readyAckCount.get()
            Log.i(TAG, "Got $acks/$blePeers READY_ACKs, proceeding")

            // Step 3: Start native audio (mic -> ADPCM -> transport)
            SassyTalkNative.pttStart()
            Log.i(TAG, "Native PTT started")

            // Step 4: Start RFCOMM TX pump
            if (rfcommPeers > 0) {
                btTransport.startTxPump()
            }
        }
    }

    fun onPttReleased() {
        if (!transmitting.getAndSet(false)) return

        Log.i(TAG, "PTT RELEASED")

        // Stop native audio
        SassyTalkNative.pttStop()

        // Stop RFCOMM TX
        btTransport.stopTxPump()

        // BLE signal to peers
        bleSignaling.broadcastPttStop()
    }

    // —— RX Side (Peer presses PTT, we receive) ——

    override fun onPttStartReceived(deviceAddress: String) {
        Log.i(TAG, "\u2190 PTT_START from $deviceAddress")

        // Send READY_ACK back
        bleSignaling.sendReadyAck(deviceAddress)

        // RFCOMM RX is already running (started on connect)
        // Audio will arrive and be decoded by the RX thread
    }

    override fun onPttStopReceived(deviceAddress: String) {
        Log.i(TAG, "\u2190 PTT_STOP from $deviceAddress")
        // TODO: Play roger beep
    }

    override fun onReadyAckReceived(deviceAddress: String) {
        val count = readyAckCount.incrementAndGet()
        Log.i(TAG, "\u2190 READY_ACK from $deviceAddress (total: $count)")
    }

    override fun onPeerDiscovered(device: android.bluetooth.BluetoothDevice) {
        Log.i(TAG, "Peer discovered: ${device.name ?: device.address}")
        // Auto-connect RFCOMM data channel when BLE peer found
        if (!btTransport.isConnectedTo(device.address)) {
            btTransport.connectDevice(device)
        }
    }

    override fun onPeerLost(deviceAddress: String) {
        Log.i(TAG, "Peer lost: $deviceAddress")
    }

    fun shutdown() {
        scope.cancel()
        transmitting.set(false)
    }
}
