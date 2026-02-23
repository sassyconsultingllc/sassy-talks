package com.sassyconsulting.sassytalkie

import android.util.Log
import okhttp3.*
import okio.ByteString
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

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
        private const val PING_INTERVAL_MS = 25_000L
    }

    private val client = OkHttpClient.Builder()
        .readTimeout(0, TimeUnit.MILLISECONDS)   // No timeout for WebSocket
        .pingInterval(PING_INTERVAL_MS, TimeUnit.MILLISECONDS)
        .build()

    private var webSocket: WebSocket? = null
    private val isConnected = AtomicBoolean(false)
    private val isRunning = AtomicBoolean(false)
    private var outboundThread: Thread? = null

    /** Connect to the cellular relay. Room must be set first via SassyTalkNative.cellularSetRoom() */
    fun connect(): Boolean {
        if (isConnected.get()) {
            Log.w(TAG, "Already connected")
            return true
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
                SassyTalkNative.cellularOnConnected()
                startOutboundPump()
            }

            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                // Binary message = encrypted audio frame from relay
                SassyTalkNative.cellularOnMessage(bytes.toByteArray())
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                // Text message = control (peer_joined, peer_left, welcome, pong, etc.)
                Log.d(TAG, "Control: $text")
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                Log.i(TAG, "WebSocket closing: $code $reason")
                webSocket.close(1000, null)
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                Log.i(TAG, "WebSocket closed: $code $reason")
                onDisconnected("closed: $code $reason")
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                Log.e(TAG, "WebSocket failure: ${t.message}")
                SassyTalkNative.cellularOnError(t.message ?: "unknown error")
                onDisconnected("failure: ${t.message}")
            }
        })

        return true // Connection is async; actual status comes via onOpen
    }

    /** Disconnect from the relay */
    fun disconnect() {
        Log.i(TAG, "Disconnecting")
        stopOutboundPump()
        webSocket?.close(1000, "user disconnect")
        webSocket = null
        onDisconnected("user disconnect")
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

    /** Send a heartbeat ping to the relay (JSON control message) */
    fun sendPing() {
        webSocket?.send("""{"type":"ping"}""")
    }
}
