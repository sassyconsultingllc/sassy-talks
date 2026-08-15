// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-ZIIAOESP57VN
package com.sassyconsulting.sassytalkie

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import java.util.UUID
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Stable per-install identifier used as the peer_id on /presence and as the
 * `peer` query param on the cellular WS URL. Persists across app updates and
 * session re-imports; reset by uninstall, "Clear data", or a Keystore miss
 * after device change (prefs copied onto another handset will not decrypt).
 *
 * Not derived from PII (ANDROID_ID, IMEI). The UUID is wrapped with an
 * Android Keystore AES key so the value is device-bound when hardware-backed
 * Keystore is available. Fallback is a local UUID if Keystore is unavailable.
 */
object InstallId {
    private const val PREFS = "sassy_install"
    private const val KEY = "install_id"
    private const val KEY_WRAPPED = "install_id_wrapped"
    private const val ANDROID_KEYSTORE = "AndroidKeyStore"
    private const val KEY_ALIAS = "sassytalkie_install_binding"
    private const val GCM_IV_BYTES = 12
    private const val GCM_TAG_BITS = 128

    @Volatile private var cached: String? = null

    fun get(context: Context): String {
        cached?.let { return it }
        synchronized(this) {
            cached?.let { return it }
            val prefs = context.applicationContext
                .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            val wrapped = prefs.getString(KEY_WRAPPED, null)
            val plaintext = prefs.getString(KEY, null)
            val id = unwrapOrCreate(wrapped, plaintext).also { value ->
                val sealed = wrap(value)
                val editor = prefs.edit().putString(KEY_WRAPPED, sealed)
                if (!sealed.isNullOrBlank()) editor.remove(KEY) else editor.putString(KEY, value)
                editor.apply()
            }
            cached = id
            return id
        }
    }

    private fun unwrapOrCreate(wrapped: String?, plaintext: String?): String {
        unwrap(wrapped)?.let { return it }
        if (!plaintext.isNullOrBlank()) return plaintext
        return UUID.randomUUID().toString()
    }

    private fun wrap(id: String): String? = try {
        val key = getOrCreateKey() ?: return null
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key)
        val iv = cipher.iv
        val ciphertext = cipher.doFinal(id.toByteArray(Charsets.UTF_8))
        Base64.encodeToString(iv + ciphertext, Base64.NO_WRAP)
    } catch (_: Throwable) {
        null
    }

    private fun unwrap(wrapped: String?): String? {
        if (wrapped.isNullOrBlank()) return null
        return try {
            val raw = Base64.decode(wrapped, Base64.NO_WRAP)
            if (raw.size <= GCM_IV_BYTES) return null
            val key = getOrCreateKey() ?: return null
            val iv = raw.copyOfRange(0, GCM_IV_BYTES)
            val ciphertext = raw.copyOfRange(GCM_IV_BYTES, raw.size)
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(GCM_TAG_BITS, iv))
            String(cipher.doFinal(ciphertext), Charsets.UTF_8).takeIf { it.isNotBlank() }
        } catch (_: Throwable) {
            null
        }
    }

    private fun getOrCreateKey(): SecretKey? = try {
        val ks = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        (ks.getEntry(KEY_ALIAS, null) as? KeyStore.SecretKeyEntry)?.secretKey
            ?: run {
                val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
                gen.init(
                    KeyGenParameterSpec.Builder(
                        KEY_ALIAS,
                        KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
                    )
                        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                        .setKeySize(256)
                        .build(),
                )
                gen.generateKey()
            }
    } catch (_: Throwable) {
        null
    }
}
