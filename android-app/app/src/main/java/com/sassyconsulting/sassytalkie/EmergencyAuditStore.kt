// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import org.json.JSONArray
import org.json.JSONObject
import java.security.KeyStore
import java.security.MessageDigest
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import javax.crypto.KeyGenerator
import javax.crypto.Mac

/**
 * Local, bounded, hash-chained technical audit. Tamper-evident app state, not
 * a legal chain of custody and not court-certified evidence.
 */
class EmergencyAuditStore(context: Context) {
    private val app = context.applicationContext
    private val prefs = app.getSharedPreferences("emergency_local_state", Context.MODE_PRIVATE)

    var selfActive: Boolean
        get() = prefs.getBoolean("self_active", false)
        set(value) { prefs.edit().putBoolean("self_active", value).apply() }

    @Synchronized
    fun append(event: String, detail: String = "") {
        val entries = readEntries()
        val previous = if (entries.length() > 0) {
            entries.optJSONObject(entries.length() - 1)?.optString("hash").orEmpty()
        } else {
            ""
        }
        val ts = System.currentTimeMillis()
        val normalizedDetail = redact(detail)
        val hash = sha256("$previous|$ts|$event|$normalizedDetail")
        entries.put(
            JSONObject()
                .put("ts", ts)
                .put("ts_utc", utc(ts))
                .put("event", event)
                .put("detail", normalizedDetail)
                .put("previous", previous)
                .put("hash", hash)
        )
        val bounded = JSONArray()
        val start = (entries.length() - MAX_EVENTS).coerceAtLeast(0)
        for (i in start until entries.length()) bounded.put(entries.get(i))
        prefs.edit().putString("audit", bounded.toString()).apply()
    }

    @Synchronized
    fun clear() {
        prefs.edit().remove("audit").apply()
    }

    /** Export integrity metadata plus bounded redacted events. Not a legal chain of custody. */
    @Synchronized
    fun exportJson(): String = exportPackage()

    @Synchronized
    fun exportPackage(): String {
        val entries = readEntries()
        val firstPrevious = entries.optJSONObject(0)?.optString("previous").orEmpty()
        val head = entries.optJSONObject(entries.length() - 1)?.optString("hash").orEmpty()
        val versionName = try {
            app.packageManager.getPackageInfo(app.packageName, 0).versionName ?: ""
        } catch (_: Throwable) {
            ""
        }
        val installId = try { InstallId.get(app) } catch (_: Throwable) { "" }
        val body = JSONObject()
            .put("format", FORMAT)
            .put("disclaimer", DISCLAIMER)
            .put("hash_algorithm", "SHA-256")
            .put("exported_at_utc", utc(System.currentTimeMillis()))
            .put("app_id", app.packageName)
            .put("app_version", versionName)
            .put("install_id", installId)
            .put("event_count", entries.length())
            .put("first_previous_hash", firstPrevious)
            .put("head_hash", head)
            .put("chain_valid", verify(entries))
            .put("retention_max_events", MAX_EVENTS)
            .put("events", entries)
        val canonical = body.toString()
        val payloadHash = sha256(canonical)
        val signature = sign(payloadHash.toByteArray(Charsets.UTF_8))
        return body
            .put("manifest_hash", payloadHash)
            .put("manifest_signature", signature ?: JSONObject.NULL)
            .put("signature_alg", if (signature != null) "HmacSHA256-AndroidKeyStore" else "none")
            .toString()
    }

    @Synchronized
    fun isChainValid(): Boolean = verify(readEntries())

    private fun readEntries(): JSONArray = try {
        JSONArray(prefs.getString("audit", "[]"))
    } catch (_: Throwable) {
        JSONArray()
    }

    private fun verify(entries: JSONArray): Boolean {
        var previous = entries.optJSONObject(0)?.optString("previous").orEmpty()
        for (i in 0 until entries.length()) {
            val item = entries.optJSONObject(i) ?: return false
            if (item.optString("previous") != previous) return false
            val expected = sha256(
                "$previous|${item.optLong("ts")}|${item.optString("event")}|" +
                    item.optString("detail"),
            )
            if (item.optString("hash") != expected) return false
            previous = expected
        }
        return true
    }

    private fun sign(payload: ByteArray): String? = try {
        val key = hmacKey() ?: return null
        val mac = Mac.getInstance("HmacSHA256")
        mac.init(key)
        Base64.encodeToString(mac.doFinal(payload), Base64.NO_WRAP)
    } catch (_: Throwable) {
        null
    }

    private fun hmacKey(): java.security.Key? = try {
        val ks = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        (ks.getEntry(HMAC_ALIAS, null) as? KeyStore.SecretKeyEntry)?.secretKey
            ?: run {
                val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_HMAC_SHA256, ANDROID_KEYSTORE)
                gen.init(
                    KeyGenParameterSpec.Builder(
                        HMAC_ALIAS,
                        KeyProperties.PURPOSE_SIGN or KeyProperties.PURPOSE_VERIFY,
                    )
                        .setDigests(KeyProperties.DIGEST_SHA256)
                        .build(),
                )
                gen.generateKey()
            }
    } catch (_: Throwable) {
        null
    }

    companion object {
        const val FORMAT = "sassytalkie-technical-audit-v1"
        const val DISCLAIMER =
            "technical audit export — not a legal chain of custody / not court-certified evidence"
        const val MAX_EVENTS = 500
        private const val ANDROID_KEYSTORE = "AndroidKeyStore"
        private const val HMAC_ALIAS = "sassytalkie_audit_hmac"

        fun redact(detail: String): String {
            val tokens = detail.split(Regex("\\s+"))
            val out = StringBuilder()
            for (token in tokens) {
                val piece = if (looksLikeSecret(token)) "[redacted]" else token
                if (out.isNotEmpty()) out.append(' ')
                out.append(piece)
                if (out.length >= 256) {
                    return out.substring(0, 256)
                }
            }
            return if (out.isEmpty()) detail.take(256) else out.toString()
        }

        fun looksLikeSecret(token: String): Boolean {
            val t = token.trim(',', ';', '=')
            if (t.length >= 32 && t.all { it.isDigit() || it in 'a'..'f' || it in 'A'..'F' }) return true
            val lower = t.lowercase(Locale.US)
            return lower.startsWith("psk=") || lower.startsWith("key=") || lower.startsWith("token=")
        }

        fun utc(ts: Long): String {
            val fmt = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss.SSS'Z'", Locale.US)
            fmt.timeZone = TimeZone.getTimeZone("UTC")
            return fmt.format(Date(ts))
        }

        fun sha256(value: String): String =
            MessageDigest.getInstance("SHA-256")
                .digest(value.toByteArray(Charsets.UTF_8))
                .joinToString("") { "%02x".format(it) }
    }
}
