package com.sassyconsulting.sassytalkie

import android.util.Log
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONObject
import java.util.concurrent.TimeUnit

/**
 * Fetches short-lived HMAC capability tokens from the relay's /auth endpoint.
 *
 * The relay gates every mutating endpoint on one of these tokens:
 *   - /ws           (open a room WebSocket)      — CellularWebSocketClient
 *   - /presence     (register/unregister FCM)    — PresenceClient
 *   - /share        (create/delete an invite)    — SessionShareLink
 *
 * A token is `<expSec>.<hmacHex>` bound to a specific room id and valid for
 * ~5 minutes. `/auth` itself is unauthenticated (the room id is the capability;
 * it is derived from the QR the peers already share), so any current member can
 * mint a token for their own room.
 *
 * BLOCKING — every call performs a synchronous HTTP round-trip and must run on
 * a background thread (Dispatchers.IO or a daemon thread), never the main thread.
 */
object RelayAuth {
    private const val TAG = "RelayAuth"

    private val http: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .connectTimeout(10, TimeUnit.SECONDS)
            .readTimeout(15, TimeUnit.SECONDS)
            .build()
    }

    /**
     * GET /auth?room=<roomId> and return the capability token, or null on any
     * failure (network error, non-2xx, missing token). Blocks the caller.
     */
    fun fetchToken(roomId: String): String? {
        if (roomId.isBlank()) return null
        val url = "${SessionShareLink.RELAY_BASE}/auth".toHttpUrlOrNull()
            ?.newBuilder()
            ?.setQueryParameter("room", roomId)
            ?.build()
            ?: return null
        val req = Request.Builder().url(url).get().build()
        return try {
            http.newCall(req).execute().use { r ->
                if (!r.isSuccessful) {
                    Log.w(TAG, "auth failed for room: HTTP ${r.code}")
                    return null
                }
                val token = JSONObject(r.body?.string() ?: "").optString("token")
                token.ifBlank { null }
            }
        } catch (t: Throwable) {
            Log.w(TAG, "auth exception: ${t.message}")
            null
        }
    }
}
