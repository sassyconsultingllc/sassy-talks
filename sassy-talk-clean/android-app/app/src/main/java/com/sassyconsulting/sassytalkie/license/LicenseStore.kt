package com.sassyconsulting.sassytalkie.license

import android.content.Context
import android.content.SharedPreferences
import android.util.Log
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

/**
 * Keystore-backed storage for the entitlement state written by the per-flavor
 * [Entitlements] object (Play purchase cache or direct-license receipt).
 *
 * Encrypted for the same reason session keys are: a casual `run-as`/backup
 * prefs edit shouldn't mint a free unlock. A rooted device can defeat any
 * client-side gate — that's an accepted non-goal for a $2 app.
 */
object LicenseStore {
    private const val TAG = "LicenseStore"
    private const val PREFS = "sassy_license"

    // Shared keys. Play flavor uses UNLOCKED; direct flavor uses the rest.
    const val KEY_UNLOCKED = "unlocked"
    const val KEY_LICENSE = "license_key"
    const val KEY_RECEIPT_EXP = "receipt_exp"

    @Volatile private var cached: SharedPreferences? = null

    fun prefs(context: Context): SharedPreferences? {
        cached?.let { return it }
        return try {
            val masterKey = MasterKey.Builder(context.applicationContext)
                .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                .build()
            EncryptedSharedPreferences.create(
                context.applicationContext,
                PREFS,
                masterKey,
                EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
                EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
            ).also { cached = it }
        } catch (e: Exception) {
            // Keystore unavailable (rare vendor bugs). Entitlement will be
            // re-derived from Play / the license server on next launch.
            Log.w(TAG, "EncryptedSharedPreferences unavailable: ${e.message}")
            null
        }
    }
}
