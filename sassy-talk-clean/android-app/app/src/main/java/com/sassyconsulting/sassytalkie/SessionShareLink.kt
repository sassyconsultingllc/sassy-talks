package com.sassyconsulting.sassytalkie

import android.net.Uri
import android.util.Base64
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import java.security.SecureRandom
import java.util.concurrent.TimeUnit
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

/**
 * Client for the relay's encrypted session-share endpoint.
 *
 * Wire contract (must match cloudflare-worker/src/share.js exactly):
 *   POST   /share?room=<session_id>[&ttl=<sec>]   body = raw blob bytes
 *          Authorization: Bearer <room capability token>   → { id, expires_at }
 *   GET    /share/<id>                            → raw blob bytes (octet-stream)
 *
 * The blob the worker stores is opaque ciphertext it cannot read:
 *   blob = iv(12) || AES-256-GCM(ciphertext + tag)
 * The 256-bit key travels only in the share link's URL #fragment, which is
 * never transmitted to the server. So the relay sees a room-bound ciphertext
 * blob and the recipient supplies the key out of band via the link.
 *
 * Share link: https://relay.sassyconsultingllc.com/share/<id>#<base64url-key>
 *
 * CREATE requires a capability token for the room the invite belongs to (the
 * shared session_id) — this blocks anonymous storage abuse. GET is open: the
 * unguessable 128-bit id is the bearer capability, and the blob is useless
 * without the #fragment key.
 */
object SessionShareLink {

    const val RELAY_BASE = "https://relay.sassyconsultingllc.com"

    private const val GCM_TAG_BITS = 128
    private const val IV_LEN = 12
    private val OCTET_MEDIA = "application/octet-stream".toMediaType()

    // Worker share ids are base64url, 16–64 chars (16 random bytes → 22 chars).
    private val SHARE_ID_RE = Regex("^[A-Za-z0-9_-]{16,64}$")

    private val http: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .connectTimeout(10, TimeUnit.SECONDS)
            .readTimeout(15, TimeUnit.SECONDS)
            .build()
    }

    sealed class Result {
        /**
         * [url] / [expiresAt] / [ttlSec] are populated for [createShare].
         * [json] is populated for [importFromShareUri] and contains the
         * decrypted session payload ready to hand to [SassyTalkNative.importSession].
         */
        data class Ok(
            val url: String = "",
            val expiresAt: Long = 0L,
            val ttlSec: Int = 0,
            val json: String = "",
        ) : Result()
        data class Err(val message: String) : Result()
    }

    /**
     * Encrypt [sessionJson], upload the ciphertext bound to its room, and
     * return a share URL. Blocks — call from a coroutine on Dispatchers.IO.
     *
     * The room is the `session_id` carried inside [sessionJson]; the caller
     * (a current member/host) can mint a capability token for it via /auth.
     */
    fun createShare(sessionJson: String, ttlSec: Int? = null): Result {
        if (sessionJson.isEmpty()) return Result.Err("Empty session payload")

        // The invite is bound to the session's room so the relay can authorize
        // the upload against a capability the creator already holds.
        val roomId = try {
            JSONObject(sessionJson).optString("session_id")
        } catch (t: Throwable) {
            return Result.Err("Session payload is not valid JSON")
        }
        if (roomId.isBlank()) return Result.Err("Session payload missing session_id")

        val capToken = RelayAuth.fetchToken(roomId)
            ?: return Result.Err("Could not authorize share (relay unreachable?)")

        val keyBytes = ByteArray(32)
        SecureRandom().nextBytes(keyBytes)
        val iv = ByteArray(IV_LEN)
        SecureRandom().nextBytes(iv)

        val ciphertext: ByteArray = try {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(
                Cipher.ENCRYPT_MODE,
                SecretKeySpec(keyBytes, "AES"),
                GCMParameterSpec(GCM_TAG_BITS, iv),
            )
            cipher.doFinal(sessionJson.toByteArray(Charsets.UTF_8))
        } catch (t: Throwable) {
            keyBytes.fill(0)
            return Result.Err("Encrypt failed: ${t.message}")
        }

        // Blob layout the worker stores verbatim: iv(12) || ciphertext+tag.
        val blob = iv + ciphertext

        val postUrl = "$RELAY_BASE/share".toHttpUrlOrNull()!!
            .newBuilder()
            .setQueryParameter("room", roomId)
            .apply { if (ttlSec != null) setQueryParameter("ttl", ttlSec.toString()) }
            .build()

        val req = Request.Builder()
            .url(postUrl)
            .addHeader("Authorization", "Bearer $capToken")
            .post(blob.toRequestBody(OCTET_MEDIA))
            .build()

        val (id, expiresAt) = try {
            http.newCall(req).execute().use { resp ->
                if (!resp.isSuccessful) {
                    keyBytes.fill(0)
                    return Result.Err("Server returned HTTP ${resp.code}")
                }
                val json = JSONObject(resp.body?.string() ?: "")
                json.optString("id") to json.optLong("expires_at", 0L)
            }
        } catch (t: Throwable) {
            keyBytes.fill(0)
            return Result.Err("Network error: ${t.message}")
        }

        if (id.isNullOrBlank()) {
            keyBytes.fill(0)
            return Result.Err("Server returned no id")
        }

        val keyB64Url = Base64.encodeToString(
            keyBytes,
            Base64.NO_PADDING or Base64.NO_WRAP or Base64.URL_SAFE,
        )
        // Zero the key bytes — they're now in the URL, no reason to keep them
        // in memory longer than needed.
        keyBytes.fill(0)

        val now = System.currentTimeMillis() / 1000
        return Result.Ok(
            url = "$RELAY_BASE/share/$id#$keyB64Url",
            expiresAt = expiresAt,
            ttlSec = if (expiresAt > now) (expiresAt - now).toInt() else 0,
        )
    }

    /**
     * Resolve a deep-link Uri to a decrypted session JSON, ready to feed into
     * [SassyTalkNative.importSession]. Blocking — call from a coroutine.
     *
     * Only accepts the encrypted https /share/<id>#<key> form. A previous draft
     * supported sassytalk://join#<urlencoded-json>, but that put plaintext
     * session JSON (including the AES-256 room key) into any URL the user
     * tapped — a phishing one-tap force-join into an attacker-controlled
     * session. Removed.
     */
    fun importFromShareUri(uri: Uri): Result {
        if (uri.scheme != "https" || uri.host != "relay.sassyconsultingllc.com") {
            return Result.Err("Unrecognized share URL")
        }
        val path = uri.path ?: ""
        if (!path.startsWith("/share/")) return Result.Err("Not a share URL")
        val id = path.removePrefix("/share/")
        if (!SHARE_ID_RE.matches(id)) return Result.Err("Malformed share id")

        val keyB64 = uri.fragment
            ?: return Result.Err("Missing decryption key in URL fragment")

        val keyBytes = try {
            Base64.decode(keyB64, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
        } catch (t: Throwable) {
            return Result.Err("Invalid key encoding")
        }
        if (keyBytes.size != 32) {
            keyBytes.fill(0)
            return Result.Err("Key must be 32 bytes")
        }

        val fetchReq = Request.Builder()
            .url("$RELAY_BASE/share/$id")
            .get()
            .build()

        val blob: ByteArray = try {
            http.newCall(fetchReq).execute().use { resp ->
                when {
                    resp.code == 404 -> { keyBytes.fill(0); return Result.Err("Invite already used or expired") }
                    resp.code == 429 -> { keyBytes.fill(0); return Result.Err("Too many requests; try later") }
                    !resp.isSuccessful -> { keyBytes.fill(0); return Result.Err("Server returned HTTP ${resp.code}") }
                    else -> resp.body?.bytes() ?: ByteArray(0)
                }
            }
        } catch (t: Throwable) {
            keyBytes.fill(0)
            return Result.Err("Network error: ${t.message}")
        }

        // Blob is iv(12) || ciphertext+tag. A GCM tag is 16 bytes, so anything
        // shorter than iv + tag can't be a valid payload.
        if (blob.size < IV_LEN + 16) {
            keyBytes.fill(0)
            return Result.Err("Server response too short")
        }
        val iv = blob.copyOfRange(0, IV_LEN)
        val ciphertext = blob.copyOfRange(IV_LEN, blob.size)

        val plain = try {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(
                Cipher.DECRYPT_MODE,
                SecretKeySpec(keyBytes, "AES"),
                GCMParameterSpec(GCM_TAG_BITS, iv),
            )
            String(cipher.doFinal(ciphertext), Charsets.UTF_8)
        } catch (t: Throwable) {
            return Result.Err("Decryption failed (key wrong or payload tampered)")
        } finally {
            keyBytes.fill(0)
        }

        return Result.Ok(url = uri.toString(), json = plain)
    }
}
