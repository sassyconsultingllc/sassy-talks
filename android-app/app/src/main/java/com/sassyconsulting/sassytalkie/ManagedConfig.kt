// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import android.content.Context
import android.content.RestrictionsManager
import android.os.Bundle

/**
 * Effective enterprise policy without copying managed values into user prefs.
 * Removing device policy therefore restores the operator's prior choices.
 *
 * Work-profile / MDM deployments are supported. Restrictions are honored;
 * the app does not block managed profiles.
 */
object ManagedConfig {
    const val PREFS = "sassy_settings"
    const val KEY_LOCK_SCREEN_PTT = "lock_screen_ptt"
    const val KEY_ENABLE_WIFI = "enable_wifi_multicast"
    const val KEY_ENABLE_RELAY = "enable_cloudflare_relay"
    const val KEY_REQUIRE_RELAY = "require_relay"
    const val KEY_ENABLE_BLUETOOTH = "enable_bluetooth"
    const val KEY_ENABLE_NOTIFICATIONS = "enable_notifications"
    const val KEY_ENABLE_TRANSLATION = "enable_translation"
    const val KEY_ENABLE_DIAGNOSTICS = "enable_diagnostics_overlay"
    const val KEY_MAX_TX_SECONDS = "max_tx_seconds"
    const val KEY_SESSION_IDLE_TIMEOUT = "session_idle_timeout_minutes"
    const val KEY_ENROLLMENT_TOKEN = "enrollment_token"
    const val KEY_OPERATOR_ROLE = "operator_role"
    const val KEY_FORCE_SESSION_WIPE = "force_session_wipe"
    const val KEY_REQUIRE_FIPS = "require_fips_provider"
    const val KEY_REQUIRE_TLS_PINNING = "require_tls_pinning"

    const val DEFAULT_MAX_TX_SECONDS = 60
    /**
     * Unmanaged devices allow the debug HUD. MDM sets
     * `enable_diagnostics_overlay=false` to force it off. Default true so a
     * debug build's radio overlay is not silently suppressed.
     */
    const val DEFAULT_DIAGNOSTICS_ALLOWED = true
    const val ROLE_OPERATOR = "operator"
    const val ROLE_SUPERVISOR = "supervisor"

    val ALL_KEYS = listOf(
        KEY_LOCK_SCREEN_PTT,
        KEY_ENABLE_WIFI,
        KEY_ENABLE_RELAY,
        KEY_REQUIRE_RELAY,
        KEY_ENABLE_BLUETOOTH,
        KEY_ENABLE_NOTIFICATIONS,
        KEY_ENABLE_TRANSLATION,
        KEY_ENABLE_DIAGNOSTICS,
        KEY_MAX_TX_SECONDS,
        KEY_SESSION_IDLE_TIMEOUT,
        KEY_ENROLLMENT_TOKEN,
        KEY_OPERATOR_ROLE,
        KEY_FORCE_SESSION_WIPE,
        KEY_REQUIRE_FIPS,
        KEY_REQUIRE_TLS_PINNING,
    )

    fun restrictions(context: Context): Bundle? = try {
        (context.getSystemService(Context.RESTRICTIONS_SERVICE) as? RestrictionsManager)
            ?.applicationRestrictions
    } catch (_: Throwable) {
        null
    }

    fun isManagedKey(restrictions: Bundle?, key: String): Boolean =
        restrictions?.containsKey(key) == true

    fun boolean(context: Context, key: String, default: Boolean): Boolean {
        val restrictions = restrictions(context)
        if (restrictions?.containsKey(key) == true) return restrictions.getBoolean(key, default)
        return context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(key, default)
    }

    fun int(context: Context, key: String, default: Int): Int {
        val restrictions = restrictions(context)
        if (restrictions?.containsKey(key) == true) return restrictions.getInt(key, default)
        return context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getInt(key, default)
    }

    fun string(context: Context, key: String, default: String = ""): String {
        val restrictions = restrictions(context)
        if (restrictions?.containsKey(key) == true) {
            return restrictions.getString(key, default) ?: default
        }
        return context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getString(key, default) ?: default
    }

    fun maxTxMs(context: Context): Long {
        val seconds = int(context, KEY_MAX_TX_SECONDS, DEFAULT_MAX_TX_SECONDS)
        return seconds.coerceIn(10, 300) * 1_000L
    }

    fun requireRelay(context: Context): Boolean = boolean(context, KEY_REQUIRE_RELAY, false)

    fun translationAllowed(context: Context): Boolean = boolean(context, KEY_ENABLE_TRANSLATION, true)

    fun diagnosticsAllowed(context: Context): Boolean =
        boolean(context, KEY_ENABLE_DIAGNOSTICS, DEFAULT_DIAGNOSTICS_ALLOWED)

    fun notificationsAllowed(context: Context): Boolean = boolean(context, KEY_ENABLE_NOTIFICATIONS, true)

    fun lockScreenPttActionsAllowed(lockScreenPref: Boolean, notificationsAllowed: Boolean): Boolean =
        lockScreenPref && notificationsAllowed

    fun lockScreenPttActionsAllowed(context: Context): Boolean =
        lockScreenPttActionsAllowed(
            boolean(context, KEY_LOCK_SCREEN_PTT, false),
            notificationsAllowed(context),
        )

    fun enrollmentToken(context: Context): String = string(context, KEY_ENROLLMENT_TOKEN, "")

    fun operatorRole(context: Context): String {
        val raw = string(context, KEY_OPERATOR_ROLE, ROLE_OPERATOR).trim().lowercase()
        return if (raw == ROLE_SUPERVISOR) ROLE_SUPERVISOR else ROLE_OPERATOR
    }

    fun canInvokeWipe(context: Context): Boolean = operatorRole(context) == ROLE_SUPERVISOR

    fun forceSessionWipe(context: Context): Boolean = boolean(context, KEY_FORCE_SESSION_WIPE, false)

    fun requireFipsProvider(context: Context): Boolean = boolean(context, KEY_REQUIRE_FIPS, false)

    /** Relay SPKI pinning. Default on when the backup pin-set is complete. MDM false disables. */
    fun tlsPinningEnabled(context: Context): Boolean =
        boolean(context, KEY_REQUIRE_TLS_PINNING, RelayTlsPins.productionDefaultEnabled)

    fun sessionIdleTimeoutMs(context: Context): Long {
        val minutes = int(context, KEY_SESSION_IDLE_TIMEOUT, 0)
        if (minutes <= 0) return 0L
        return minutes.coerceIn(1, 24 * 60) * 60_000L
    }

    /** When relay is required, local Wi-Fi / BT must not be used. */
    fun wifiEnabled(context: Context): Boolean {
        if (requireRelay(context)) return false
        return boolean(context, KEY_ENABLE_WIFI, true)
    }

    fun bluetoothEnabled(context: Context): Boolean {
        if (requireRelay(context)) return false
        return boolean(context, KEY_ENABLE_BLUETOOTH, true)
    }

    fun relayEnabled(context: Context): Boolean {
        if (requireRelay(context)) return true
        return boolean(context, KEY_ENABLE_RELAY, true)
    }
}
