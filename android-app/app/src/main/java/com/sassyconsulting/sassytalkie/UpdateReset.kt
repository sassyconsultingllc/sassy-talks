// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

/**
 * Post-update state reset — "fresh install" behaviour when the app version
 * changes, without the collateral damage of an actual wipe.
 *
 * WHY: an in-place update keeps every byte of app state, but the binary
 * underneath it changed. The native library is new, its counters and caches
 * describe a process that no longer exists, the relay socket is bound to the
 * old process's room, and cached audio was produced by the previous codec
 * build. Reinstalling clears all of that, which is why "just reinstall it"
 * keeps working as a fix — the update path simply never did the cleanup.
 *
 * WHAT IS DELIBERATELY NOT CLEARED by default: session keys, the user profile
 * and the entitlement receipt. A true wipe would log every user out of their
 * channel on every Play update and force a QR re-pair with whoever hosts them
 * — for a group of field users that is worse than the staleness it fixes, and
 * it would defeat the stored-credential rejoin path entirely. Set
 * [KEY_FULL_RESET] to get literal fresh-install semantics anyway; it is off by
 * default because the cost lands on the user, not the developer.
 *
 * HOW IT TRIGGERS: primarily by comparing the stored version code against
 * [BuildConfig.VERSION_CODE] at startup, NOT by the broadcast alone.
 * `ACTION_MY_PACKAGE_REPLACED` is not delivered to an app the user has
 * force-stopped, and OEM battery managers force-stop aggressively — a
 * receiver-only design silently misses exactly the devices most likely to
 * need the reset. [AppUpdateReceiver] is an accelerator that lets the work
 * happen before the user opens the app; the startup check is the guarantee.
 */
object UpdateReset {

    private const val TAG = "UpdateReset"
    private const val PREFS = "sassy_settings"

    /** Last version code that completed a post-update reset. */
    const val KEY_LAST_VERSION = "last_seen_version_code"

    /**
     * Opt-in: also clear session keys and cohort history on update — literal
     * fresh-install semantics. Off by default; see the class note on why.
     */
    const val KEY_FULL_RESET = "update_full_reset"

    /**
     * Pure decision so the trigger logic is testable without Android.
     *
     * First run (`last == 0`) is NOT an update — there is nothing stale to
     * clear on a genuine first install, and running the reset there would just
     * add startup work. A DOWNGRADE (last > current) counts: the state was
     * written by a newer binary and is exactly as suspect as a stale one.
     */
    fun isVersionTransition(last: Int, current: Int): Boolean =
        last != 0 && last != current

    /**
     * Run the reset if the version changed. Idempotent and cheap when it is a
     * no-op — one int comparison — so it is safe on every startup path.
     *
     * @return true when a reset actually ran.
     */
    fun runIfUpdated(context: Context): Boolean {
        val prefs = context.applicationContext
            .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val last = prefs.getInt(KEY_LAST_VERSION, 0)
        val current = BuildConfig.VERSION_CODE

        if (!isVersionTransition(last, current)) {
            // Record the first-install version so the NEXT update is detected.
            if (last != current) prefs.edit().putInt(KEY_LAST_VERSION, current).apply()
            return false
        }

        val full = prefs.getBoolean(KEY_FULL_RESET, false)
        Log.w(TAG, "Version transition $last -> $current — post-update reset (full=$full)")

        // Derived state: produced by the previous binary, describes a process
        // that no longer exists. Always cleared.
        runCatching { SassyTalkNative.clearAudioCache() }
            .onFailure { Log.w(TAG, "clearAudioCache: ${it.message}") }
        runCatching { TranscriptionBridge.clearEntries() }
            .onFailure { Log.w(TAG, "clearEntries: ${it.message}") }
        runCatching { SassyTalkNative.diagReset() }
            .onFailure { Log.w(TAG, "diagReset: ${it.message}") }

        if (full) {
            // Literal fresh install: the user re-pairs by QR after this.
            Log.w(TAG, "FULL reset — clearing session keys and cohort history")
            runCatching { SassyTalkNative.clearSession() }
                .onFailure { Log.w(TAG, "clearSession: ${it.message}") }
            runCatching { SassyTalkNative.clearCohortHistory() }
                .onFailure { Log.w(TAG, "clearCohortHistory: ${it.message}") }
        }

        prefs.edit().putInt(KEY_LAST_VERSION, current).apply()
        Log.i(TAG, "Post-update reset complete at version $current")
        return true
    }
}

/**
 * Fires the post-update reset as soon as Play replaces the package, so the
 * work is already done before the user next opens the app.
 *
 * This is an optimisation, not the mechanism — see [UpdateReset] on why the
 * startup version check is the actual guarantee. `ACTION_MY_PACKAGE_REPLACED`
 * is only ever delivered to the app whose own package was replaced, so this
 * receiver cannot be triggered by another app.
 */
class AppUpdateReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_MY_PACKAGE_REPLACED) return
        Log.i("UpdateReset", "ACTION_MY_PACKAGE_REPLACED — running post-update reset")
        // Native may not be initialised in this process yet; every step is
        // individually guarded, and the startup check re-runs the reset if the
        // version marker did not get written here.
        runCatching { UpdateReset.runIfUpdated(context) }
            .onFailure { Log.w("UpdateReset", "receiver reset failed: ${it.message}") }
    }
}
