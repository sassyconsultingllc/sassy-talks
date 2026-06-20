package com.sassyconsulting.sassytalkie

import android.util.Log
import okhttp3.*
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okio.ByteString
import org.json.JSONObject
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
        // Outbound pump idle-poll interval. Each frame adds up to this many
        // milliseconds of send latency beyond capture-to-encode time, and the
        // receiver has no large jitter buffer so wire-side pacing irregularity
        // is audible as "robotic / chopped" audio. 2 ms keeps the pump
        // responsive without burning CPU. The pump also drains the queue
        // fully each wake so bursts go on the wire immediately.
        private const val POLL_INTERVAL_MS = 2L
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

    /**
     * Stable per-install peer identifier appended to the WS URL as `peer=...`.
     * The relay uses it to map WS sessions to /presence registrations so audio
     * for a peer with no active WS triggers an FCM wake push. Set this from
     * the wire-up site (AutoConnectManager) via InstallId.get(context).
     */
    var peerId: String? = null

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

        val baseWsUrl = SassyTalkNative.cellularGetWsUrl()
        if (baseWsUrl.isBlank()) {
            Log.e(TAG, "No WS URL — set room first")
            return false
        }

        // Fetch the capability token asynchronously. Doing this synchronously
        // here would block the calling thread (typically Dispatchers.Main via
        // AutoConnectManager.scope.launch) and throw NetworkOnMainThreadException,
        // which manifested as the cell transport flipping CONNECTED→---→CONNECTED
        // every reconnect cycle. enqueue() runs on the OkHttp dispatcher.
        Log.i(TAG, "Fetching relay auth token…")
        authorizeWsUrlAsync(baseWsUrl) { wsUrl, err ->
            if (err != null || wsUrl == null) {
                Log.e(TAG, "Auth fetch failed: ${err?.message}")
                SassyTalkNative.cellularOnError("auth: ${err?.message ?: "unknown"}")
                scheduleReconnect("auth failure: ${err?.message}")
                return@authorizeWsUrlAsync
            }
            openWebSocketAuthenticated(wsUrl)
        }

        return true // Connection is async; status comes via onOpen
    }

    private fun openWebSocketAuthenticated(wsUrl: String) {
        Log.i(TAG, "Connecting to relay (authenticated)")

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
                            // Derive a per-peer routing key from the TLV payload
                            // when possible. Using a constant "relay" key
                            // collapsed every cellular peer into one entry in
                            // PttCoordinator's LivenessTracker, so the Users
                            // tab showed N real peers as a single pseudo-peer
                            // and presence/health was meaningless.
                            val peerId = relayPeerIdFromFrame(raw) ?: "relay"
                            pttCoordinator?.onControlFrame(peerId, raw)
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
                    // Drain ALL queued packets per wake — bursts (e.g. from
                    // PTT release buffer-mode, or post-jitter catch-up) go
                    // out in one tight loop without paying the poll
                    // interval per frame. Only after the queue is empty do
                    // we sleep, and even then only briefly.
                    var sentAny = false
                    while (isRunning.get() && isConnected.get()) {
                        val packet = SassyTalkNative.cellularPollOutbound() ?: break
                        if (packet.isEmpty()) break
                        webSocket?.send(ByteString.of(*packet))
                        sentAny = true
                    }
                    if (!sentAny) {
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

    /**
     * Fetch an HMAC-signed capability token from the relay's /auth endpoint
     * (async, off the calling thread), then invoke [onResult] with the WS URL
     * carrying the appended token. On failure, [onResult] is called with
     * (null, exception).
     *
     * MUST be async — the caller (CellularWebSocketClient.connect) is invoked
     * from the AutoConnectManager coroutine on Dispatchers.Main, so a blocking
     * execute() throws NetworkOnMainThreadException.
     */
    private fun authorizeWsUrlAsync(baseWsUrl: String, onResult: (String?, Throwable?) -> Unit) {
        val httpUrl = toHttpScheme(baseWsUrl).toHttpUrlOrNull()
        if (httpUrl == null) {
            onResult(null, IllegalStateException("Invalid WS URL: $baseWsUrl"))
            return
        }
        val room = httpUrl.queryParameter("room")
        if (room.isNullOrBlank()) {
            onResult(null, IllegalStateException("WS URL missing room param"))
            return
        }

        val authUrl = httpUrl.newBuilder()
            .encodedPath("/auth")
            .build()
            .newBuilder()
            .setQueryParameter("room", room)
            .build()

        val req = Request.Builder().url(authUrl).get().build()
        client.newCall(req).enqueue(object : Callback {
            override fun onFailure(call: Call, e: java.io.IOException) {
                onResult(null, e)
            }

            override fun onResponse(call: Call, response: Response) {
                response.use { resp ->
                    try {
                        if (!resp.isSuccessful) {
                            onResult(null, IllegalStateException("auth http ${resp.code}"))
                            return
                        }
                        val bodyText = resp.body?.string()
                        if (bodyText.isNullOrEmpty()) {
                            onResult(null, IllegalStateException("auth empty body"))
                            return
                        }
                        val token = JSONObject(bodyText).optString("token")
                        if (token.isBlank()) {
                            onResult(null, IllegalStateException("auth token missing"))
                            return
                        }
                        val builder = toHttpScheme(baseWsUrl).toHttpUrlOrNull()!!
                            .newBuilder()
                            .setQueryParameter("token", token)
                        // Stable peer-id so the relay can match this WS to a
                        // /presence row and skip FCM pushes when we're online.
                        peerId?.takeIf { it.isNotBlank() }?.let {
                            builder.setQueryParameter("peer", it)
                        }
                        val authedUrl = builder.build().toString().let { toWsScheme(it) }
                        onResult(authedUrl, null)
                    } catch (t: Throwable) {
                        onResult(null, t)
                    }
                }
            }
        })
    }

    /** OkHttp's HttpUrl parser rejects ws://, so swap to http:// for parsing only. */
    private fun toHttpScheme(url: String): String = when {
        url.startsWith("wss://") -> "https://" + url.removePrefix("wss://")
        url.startsWith("ws://")  -> "http://"  + url.removePrefix("ws://")
        else -> url
    }

    private fun toWsScheme(url: String): String = when {
        url.startsWith("https://") -> "wss://" + url.removePrefix("https://")
        url.startsWith("http://")  -> "ws://"  + url.removePrefix("http://")
        else -> url
    }

    /**
     * Extract a stable per-peer routing key from a TLV control frame. The
     * relay broadcasts frames from N peers down a single WebSocket, so we
     * can't use a per-connection identity; we have to look inside the
     * frame. Five of the seven epoch-bearing opcodes (HEARTBEAT, RECV_ACK,
     * EOT_ACK, PTT_START_V2, PTT_STOP_V2) start their payload with the
     * sender's `epoch:i64` (little-endian) — a 64-bit random session id
     * generated once per app start. That's unique enough to use as a
     * LivenessTracker key.
     *
     * Returns "relay:<epoch>" for the five epoch-prefixed ops, or null for
     * frames whose payload does NOT start with epoch (CAPABILITIES, which
     * uses JSON; PARTNER_OFFLINE, which starts with a length byte). Callers
     * fall back to the legacy constant "relay" key in those cases — the
     * tracker entry is just slightly less precise for two infrequent ops.
     */
    private fun relayPeerIdFromFrame(raw: ByteArray): String? {
        if (raw.isEmpty()) return null
        val op = raw[0].toInt() and 0xFF

        // PARTNER_OFFLINE is the only frame that names the peer that left.
        // Layout: [op][len:u16 LE][peer_id_len:u8][peer_id bytes...]
        // Route it under the leaving peer's own per-epoch id is not possible
        // (we only get the server-assigned UUID here), but we CAN at least
        // distinguish offline events for different peers by using their
        // name as the routing key so liveness.removePeer targets correctly.
        if (op == 0x14) {
            if (raw.size < 4) return null
            val idLen = raw[3].toInt() and 0xFF
            if (raw.size < 4 + idLen || idLen == 0) return null
            val name = String(raw, 4, idLen, Charsets.UTF_8)
            return "relay:peer:$name"
        }

        // Epoch-prefixed opcodes: payload starts with i64 LE epoch of the
        // SENDER. Use that as a stable per-peer routing key over the WS.
        // OP_WAKE (0x17) was added in Phase-1 wake-beacon; if it's omitted
        // here every cellular WAKE collapses to the constant 'relay' key
        // and the wake's per-peer epoch tracking goes blind.
        if (raw.size < 3 + 8) return null
        if (op !in setOf(0x10, 0x11, 0x12, 0x15, 0x16, 0x17)) return null
        val bb = java.nio.ByteBuffer.wrap(raw, 3, 8).order(java.nio.ByteOrder.LITTLE_ENDIAN)
        val epoch = bb.long
        // SessionEpoch.generate() never produces 0; treat 0 as "absent" and
        // fall through to the constant key rather than collapse onto a
        // confusing pseudo-id.
        if (epoch == 0L) return null
        return "relay:$epoch"
    }
}
