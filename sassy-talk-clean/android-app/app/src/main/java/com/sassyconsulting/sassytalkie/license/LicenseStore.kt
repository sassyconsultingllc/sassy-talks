package com.sassyconsulting.sassytalkie.license

import android.annotation.SuppressLint
import android.content.Context
import android.content.SharedPreferences
import android.provider.Settings
import android.util.Log
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import java.util.UUID

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
    // "license" | "promo" — which server endpoint refreshes the receipt.
    const val KEY_KIND = "credential_kind"
    // Play flavor: unix-seconds of the first refresh that saw the purchase as
    // unowned. Drives a grace window before revoking so a transient empty Play
    // query can't strand a paying user (see Entitlements.noteUnownedAndMaybeRevoke).
    const val KEY_UNOWNED_SINCE = "unowned_since"
    // Random per-install id, persisted as a stable device id when ANDROID_ID is
    // null/blank (see deviceId) so distinct installs don't share one slot.
    const val KEY_INSTALL_ID = "install_id"

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

    /**
     * Stable per-install device id for license/promo device-slot accounting.
     * Prefers ANDROID_ID; when that is null/blank (some devices, work profiles)
     * falls back to a persisted random UUID so distinct installs don't all
     * collapse onto a shared constant "unknown" and fight over one slot.
     */
    @SuppressLint("HardwareIds")
    fun deviceId(context: Context): String {
        val androidId = Settings.Secure.getString(
            context.applicationContext.contentResolver,
            Settings.Secure.ANDROID_ID,
        )
        if (!androidId.isNullOrBlank() && androidId != "unknown") return androidId
        val p = prefs(context)
        p?.getString(KEY_INSTALL_ID, null)?.let { return it }
        val fresh = UUID.randomUUID().toString()
        p?.edit()?.putString(KEY_INSTALL_ID, fresh)?.apply()
        return fresh
    }
}
