package com.sassyconsulting.sassytalkie

import android.net.Uri
import android.util.Base64
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
 * Client for the encrypted session-share endpoint.
 *
 * Flow:
 *   1. Generate a fresh AES-256 key and 12-byte IV
 *   2. AES-GCM encrypt the QR JSON payload
 *   3. POST {c, i} to https://relay.sassyconsultingllc.com/share — returns a token
 *   4. Build URL: https://relay.sassyconsultingllc.com/v/<token>#<base64url-key>
 *
 * The fragment (#) never leaves the recipient's browser → the relay only ever
 * sees ciphertext, and a KV dump would be useless without the URL.
 */
object SessionShareLink {

    const val RELAY_BASE = "https://relay.sassyconsultingllc.com"

    private const val GCM_TAG_BITS = 128
    private const val IV_LEN = 12
    private val JSON_MEDIA = "application/json".toMediaType()

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
     * Encrypt [sessionJson] and POST it. Blocks — call from a coroutine on
     * Dispatchers.IO, or wrap in [kotlinx.coroutines.withContext].
     */
    fun createShare(sessionJson: String, ttlSec: Int? = null): Result {
        if (sessionJson.isEmpty()) return Result.Err("Empty session payload")

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
            return Result.Err("Encrypt failed: ${t.message}")
        }

        val body = JSONObject().apply {
            put("c", Base64.encodeToString(ciphertext, Base64.NO_WRAP))
            put("i", Base64.encodeToString(iv, Base64.NO_WRAP))
            if (ttlSec != null) put("ttl", ttlSec)
        }

        val req = Request.Builder()
            .url("$RELAY_BASE/share")
            .post(body.toString().toRequestBody(JSON_MEDIA))
            .build()

        val (token, expiresAt, ttl) = try {
            http.newCall(req).execute().use { resp ->
                if (!resp.isSuccessful) {
                    return Result.Err("Server returned HTTP ${resp.code}")
                }
                val json = JSONObject(resp.body?.string() ?: "")
                Triple(
                    json.optString("token"),
                    json.optLong("expires_at", 0L),
                    json.optInt("ttl", 0),
                )
            }
        } catch (t: Throwable) {
            return Result.Err("Network error: ${t.message}")
        }

        if (token.isNullOrBlank()) return Result.Err("Server returned no token")

        val keyB64Url = Base64.encodeToString(
            keyBytes,
            Base64.NO_PADDING or Base64.NO_WRAP or Base64.URL_SAFE,
        )
        // Zero the key bytes — they're now in the URL, no reason to keep them
        // in memory longer than needed.
        keyBytes.fill(0)

        return Result.Ok(
            url = "$RELAY_BASE/v/$token#$keyB64Url",
            expiresAt = expiresAt,
            ttlSec = ttl,
        )
    }

    /**
     * Resolve a deep-link Uri to a decrypted session JSON, ready to feed into
     * [SassyTalkNative.importSession]. Blocking — call from a coroutine.
     *
     * Only accepts the encrypted https /v/<token>#<key> form. A previous draft
     * supported sassytalk://join#<urlencoded-json>, but that put plaintext
     * session JSON (including the AES-256 room key) into any URL the user
     * tapped — a phishing one-tap force-join into an attacker-controlled
     * session. Removed.
     */
    fun importFromShareUri(uri: Uri): Result {
        // https relay link — fetch ciphertext, decrypt
        if (uri.scheme == "https" && uri.host == "relay.sassyconsultingllc.com") {
            val path = uri.path ?: ""
            if (!path.startsWith("/v/")) return Result.Err("Not a share URL")
            val token = path.removePrefix("/v/")
            if (!token.matches(Regex("^[A-Z2-7]{8,16}$"))) {
                return Result.Err("Malformed share token")
            }
            val keyB64 = uri.fragment
                ?: return Result.Err("Missing decryption key in URL fragment")

            val keyBytes = try {
                Base64.decode(keyB64, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
            } catch (t: Throwable) {
                return Result.Err("Invalid key encoding")
            }
            if (keyBytes.size != 32) return Result.Err("Key must be 32 bytes")

            val fetchReq = Request.Builder()
                .url("$RELAY_BASE/s/$token")
                .get()
                .build()

            val (cB64, iB64) = try {
                http.newCall(fetchReq).execute().use { resp ->
                    if (resp.code == 404) return Result.Err("Invite already used or expired")
                    if (resp.code == 429) return Result.Err("Too many requests; try later")
                    if (!resp.isSuccessful) return Result.Err("Server returned HTTP ${resp.code}")
                    val body = JSONObject(resp.body?.string() ?: "")
                    body.optString("c") to body.optString("i")
                }
            } catch (t: Throwable) {
                return Result.Err("Network error: ${t.message}")
            }
            if (cB64.isNullOrEmpty() || iB64.isNullOrEmpty()) {
                return Result.Err("Server response missing ciphertext")
            }

            val plain = try {
                val cipher = Cipher.getInstance("AES/GCM/NoPadding")
                cipher.init(
                    Cipher.DECRYPT_MODE,
                    SecretKeySpec(keyBytes, "AES"),
                    GCMParameterSpec(GCM_TAG_BITS, Base64.decode(iB64, Base64.NO_WRAP)),
                )
                String(cipher.doFinal(Base64.decode(cB64, Base64.NO_WRAP)), Charsets.UTF_8)
            } catch (t: Throwable) {
                return Result.Err("Decryption failed (key wrong or payload tampered)")
            } finally {
                keyBytes.fill(0)
            }

            return Result.Ok(url = uri.toString(), json = plain)
        }

        return Result.Err("Unrecognized share URL")
    }
}
