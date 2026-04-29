package com.sassyconsulting.sassytalkie

import android.util.Log
import okhttp3.*
import okio.ByteString
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledExecutorService
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

/**
 * WebSocket client for cellular PTT relay.
 *
 * Architecture:
 *   Kotlin OkHttp WebSocket ↔ Cloudflare Durable Object relay ↔ other devices
 *
 * Data flow:
 *   TX: Rust send_audio() → outbound queue → pollOutbound() loop → WS.send(binary)
 *   RX: WS.onMessage(binary) → cellularOnMessage() → inbound queue → Rust receive_audio()
 *
 * The relay is a blind forwarder — all encryption/decryption happens in Rust.
 */
class CellularWebSocketClient {

    companion object {
        private const val TAG = "CellularWS"
        private const val POLL_INTERVAL_MS = 5L  // Poll outbound queue every 5ms (200 fps)
        private const val PING_INTERVAL_SEC = 15L
        private const val MAX_RECONNECT_ATTEMPTS = 8

        /**
         * OkHttpClient is expensive (dispatcher + connection pool + thread pools)
         * and explicitly designed to be shared. Creating one per instance wasted
         * threads and defeated pooling.
         */
        private val sharedClient: OkHttpClient = OkHttpClient.Builder()
            .readTimeout(0, TimeUnit.MILLISECONDS)
            .pingInterval(PING_INTERVAL_SEC, TimeUnit.SECONDS)
            .build()
    }

    private val client: OkHttpClient = sharedClient

    private var webSocket: WebSocket? = null
    private val isConnected = AtomicBoolean(false)
    private val isRunning = AtomicBoolean(false)
    private val reconnectAttempts = AtomicInteger(0)
    private var outboundThread: Thread? = null

    /**
     * Single-slot scheduler for reconnect attempts. Previously each failure
     * spawned a fresh Thread that slept and then called connect(); bursts of
     * failures could race and open multiple sockets. Using a single scheduler
     * with a cancel-before-schedule pattern ensures at most one reconnect is
     * in flight at any moment.
     */
    private val reconnectScheduler: ScheduledExecutorService =
        Executors.newSingleThreadScheduledExecutor { r ->
            Thread(r, "cellular-reconnect").apply { isDaemon = true }
        }
    private var pendingReconnect: ScheduledFuture<*>? = null

    /** Callback for DO readiness confirmation. */
    var onRelayReady: (() -> Unit)? = null

    /** PttCoordinator to route binary control frames (opcodes 0x10–0x1F). */
    var pttCoordinator: PttCoordinator? = null

    /** Connect to the cellular relay. Room must be set first via SassyTalkNative.cellularSetRoom() */
    fun connect(): Boolean {
        if (isConnected.get()) {
            Log.w(TAG, "Already connected")
            return true
        }
        // Respect build-time flag to disable cellular relay (opt-in at build)
        try {
            if (!BuildConfig.ENABLE_CELLULAR_RELAY) {
                Log.w(TAG, "Cellular relay disabled by build config")
                return false
            }
        } catch (e: Throwable) {
            // If BuildConfig is not available for any reason, proceed normally
        }

        val wsUrl = SassyTalkNative.cellularGetWsUrl()
        if (wsUrl.isBlank()) {
            Log.e(TAG, "No WS URL — set room first")
            return false
        }

        Log.i(TAG, "Connecting to $wsUrl")

        val request = Request.Builder()
            .url(wsUrl)
            .build()

        webSocket = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                Log.i(TAG, "WebSocket opened")
                isConnected.set(true)
                reconnectAttempts.set(0)
                SassyTalkNative.cellularOnConnected()
                startOutboundPump()
            }

            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                val raw = bytes.toByteArray()
                // Validate full TLV structure before routing to PttCoordinator:
                // byte[0] opcode in 0x10..0x1F, bytes[1..2] payload length (u16 LE),
                // total frame size must equal 3 + payloadLen.
                if (raw.size >= 3) {
                    val op = raw[0].toInt() and 0xFF
                    if (op in 0x10..0x1F) {
                        val payloadLen = (raw[1].toInt() and 0xFF) or ((raw[2].toInt() and 0xFF) shl 8)
                        if (raw.size == 3 + payloadLen) {
                            pttCoordinator?.onControlFrame("relay", raw)
                            return
                        }
                    }
                }
                // Otherwise treat as encrypted audio frame
                SassyTalkNative.cellularOnMessage(raw)
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                // Text message = control (peer_joined, peer_left, welcome, pong, etc.)
                Log.d(TAG, "Control: $text")
                // Parse "welcome" message as DO readiness confirmation
                if (text.contains("\"welcome\"") || text.contains("\"type\":\"welcome\"")) {
                    Log.i(TAG, "DO welcome received — relay is ready")
                    onRelayReady?.invoke()
                }
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                Log.i(TAG, "WebSocket closing: $code $reason")
                webSocket.close(1000, null)
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                Log.i(TAG, "WebSocket closed: $code $reason")
                onDisconnected("closed: $code $reason")
                // Server-initiated close (e.g. DO restart) previously left us
                // permanently disconnected. Retry via the same backoff as
                // onFailure so the radio recovers automatically.
                scheduleReconnect("graceful close $code")
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                Log.e(TAG, "WebSocket failure: ${t.message}")
                SassyTalkNative.cellularOnError(t.message ?: "unknown error")
                onDisconnected("failure: ${t.message}")
                scheduleReconnect("failure: ${t.message}")
            }
        })

        return true // Connection is async; actual status comes via onOpen
    }

    /** Disconnect from the relay */
    fun disconnect() {
        Log.i(TAG, "Disconnecting")
        // User-initiated disconnect must not be overridden by an auto-reconnect.
        cancelPendingReconnect()
        reconnectAttempts.set(MAX_RECONNECT_ATTEMPTS + 1) // poison the backoff
        stopOutboundPump()
        webSocket?.close(1000, "user disconnect")
        webSocket = null
        onDisconnected("user disconnect")
    }

    /**
     * Schedule a single pending reconnect attempt with capped exponential
     * backoff. Cancels any prior pending attempt so we never have more than
     * one reconnect in flight concurrently.
     */
    private fun scheduleReconnect(cause: String) {
        val attempt = reconnectAttempts.incrementAndGet()
        if (attempt > MAX_RECONNECT_ATTEMPTS) {
            Log.w(TAG, "Max reconnect attempts reached ($cause), giving up")
            return
        }
        val delayMs = minOf(3_000L * (1L shl (attempt - 1).coerceAtMost(4)), 60_000L)
        cancelPendingReconnect()
        pendingReconnect = reconnectScheduler.schedule({
            if (!isConnected.get()) {
                Log.i(TAG, "Reconnecting ($cause, attempt $attempt, delay ${delayMs}ms)…")
                try { connect() } catch (e: Exception) {
                    Log.w(TAG, "Reconnect attempt $attempt threw: ${e.message}")
                }
            }
        }, delayMs, TimeUnit.MILLISECONDS)
    }

    private fun cancelPendingReconnect() {
        pendingReconnect?.cancel(false)
        pendingReconnect = null
    }

    fun isConnected(): Boolean = isConnected.get()

    // ── Outbound pump: polls Rust queue and sends via WebSocket ──

    private fun startOutboundPump() {
        if (isRunning.getAndSet(true)) return

        outboundThread = Thread({
            Log.i(TAG, "Outbound pump started")
            while (isRunning.get() && isConnected.get()) {
                try {
                    val packet = SassyTalkNative.cellularPollOutbound()
                    if (packet != null && packet.isNotEmpty()) {
                        webSocket?.send(ByteString.of(*packet))
                    } else {
                        Thread.sleep(POLL_INTERVAL_MS)
                    }
                } catch (e: InterruptedException) {
                    break
                } catch (e: Exception) {
                    Log.e(TAG, "Outbound pump error: ${e.message}")
                    Thread.sleep(50)
                }
            }
            Log.i(TAG, "Outbound pump stopped")
        }, "cellular-outbound")
        outboundThread?.isDaemon = true
        outboundThread?.start()
    }

    private fun stopOutboundPump() {
        isRunning.set(false)
        outboundThread?.interrupt()
        outboundThread = null
    }

    private fun onDisconnected(reason: String) {
        if (isConnected.getAndSet(false)) {
            stopOutboundPump()
            SassyTalkNative.cellularOnDisconnected(reason)
        }
    }

    /** Send a raw binary frame via the WebSocket relay. */
    fun sendBinary(bytes: ByteArray) {
        webSocket?.send(ByteString.of(*bytes))
    }

    /** Send a heartbeat ping to the relay (JSON control message) */
    fun sendPing() {
        webSocket?.send("""{"type":"ping"}""")
    }
}
