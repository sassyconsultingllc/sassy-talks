package com.sassyconsulting.sassytalkie

import android.content.Context
import java.util.UUID

/**
 * Stable per-install identifier used as the peer_id on /presence and as the
 * `peer` query param on the cellular WS URL. Persists across app updates and
 * across session re-imports; reset only by uninstall or "Clear data".
 *
 * Not derived from anything PII (ANDROID_ID, IMEI, etc.) — a fresh UUID per
 * install. The relay learns the mapping (install_uuid → fcm_token), nothing
 * more.
 */
object InstallId {
    private const val PREFS = "sassy_install"
    private const val KEY = "install_id"

    @Volatile private var cached: String? = null

    fun get(context: Context): String {
        cached?.let { return it }
        synchronized(this) {
            cached?.let { return it }
            val prefs = context.applicationContext
                .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            var id = prefs.getString(KEY, null)
            if (id.isNullOrBlank()) {
                id = UUID.randomUUID().toString()
                prefs.edit().putString(KEY, id).apply()
            }
            cached = id
            return id
        }
    }
}
