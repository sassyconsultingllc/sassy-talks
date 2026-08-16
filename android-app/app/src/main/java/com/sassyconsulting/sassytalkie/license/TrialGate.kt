// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie.license

import android.content.Context
import android.util.Log

/**
 * Usage-based free trial: the paywall stays out of the way until the user has
 * actually got value out of the app.
 *
 * "Value" is deliberately defined as **a session that had at least one peer in
 * it**, not as an app launch or a session created. A walkie-talkie with nobody
 * on the other end has demonstrated nothing, and someone who opened the app
 * five times alone — or generated five QR codes that nobody scanned — has been
 * shown no reason to pay. Counting those would put the gate in front of people
 * who never heard the product work, which is the failure mode this exists to
 * avoid.
 *
 * Counting is by DISTINCT session id, so rejoining the same channel all day is
 * one session, not a drained trial. Re-entering a room you already counted
 * costs nothing.
 *
 * IMPORTANT — this is a soft gate, not a security boundary. It lives in plain
 * SharedPreferences and anyone who clears app data gets a fresh trial. That is
 * the correct trade: hardening it would mean a server round-trip on every
 * session join, which breaks the offline/mesh case the radio exists for. The
 * real entitlement check ([Entitlements]) is unchanged and still authoritative
 * for paying users.
 */
object TrialGate {

    private const val TAG = "TrialGate"
    private const val PREFS = "sassy_settings"

    /** Sessions-with-a-peer allowed before the paywall appears. */
    const val FREE_SESSIONS = 5

    /** Distinct session ids that reached "had a peer", newline-delimited. */
    private const val KEY_COUNTED_SESSIONS = "trial_counted_sessions"

    /**
     * Cap on stored ids so the pref can't grow without bound on a device that
     * joins hundreds of rooms. Only the count matters past the threshold.
     */
    private const val MAX_TRACKED = 32

    private fun prefs(context: Context) =
        context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private fun counted(context: Context): List<String> =
        prefs(context).getString(KEY_COUNTED_SESSIONS, "")
            .orEmpty()
            .split('\n')
            .filter { it.isNotBlank() }

    /** How many qualifying sessions this install has had. */
    fun qualifyingSessions(context: Context): Int = counted(context).size

    /** Sessions left before the paywall appears; 0 once the trial is spent. */
    fun sessionsRemaining(context: Context): Int =
        (FREE_SESSIONS - qualifyingSessions(context)).coerceAtLeast(0)

    /** True while the trial still has room. */
    fun inTrial(context: Context): Boolean = qualifyingSessions(context) < FREE_SESSIONS

    /**
     * The single question every gate should ask: may this install use the
     * radio right now?
     *
     * Entitled users always may. Trial users may until the trial is spent.
     * Every transport gate and the UI gate route through here so they cannot
     * drift apart — a UI that lets a trial user onto the radio while the
     * transport layer refuses to connect is worse than no trial at all.
     */
    fun mayUseRadio(context: Context): Boolean =
        Entitlements.isUnlockedCached(context) || inTrial(context)

    /**
     * Record that [sessionId] reached "at least one peer present".
     *
     * Idempotent per session id, and deliberately UNCONDITIONAL on entitlement.
     * An earlier version skipped counting for entitled users as a privacy
     * nicety; that quietly made the whole mechanism untestable, because debug
     * builds are always entitled (`BuildConfig.DEBUG` bypass in [Entitlements])
     * so nothing ever counted. It was also wrong on its own terms: if an
     * entitlement later lapses, that user should not suddenly find a pristine
     * 5-session trial waiting. Counting is bookkeeping; gating is
     * [mayUseRadio]'s job, and it checks entitlement first regardless — so
     * counting for a paying user has no user-visible effect.
     */
    fun noteQualifyingSession(context: Context, sessionId: String?) {
        if (sessionId.isNullOrBlank()) return

        val existing = counted(context)
        if (existing.contains(sessionId)) return

        val updated = (existing + sessionId).takeLast(MAX_TRACKED)
        prefs(context).edit()
            .putString(KEY_COUNTED_SESSIONS, updated.joinToString("\n"))
            .apply()

        val used = updated.size
        Log.i(
            TAG,
            "Qualifying session recorded ($used/$FREE_SESSIONS used) — " +
                "${(FREE_SESSIONS - used).coerceAtLeast(0)} free sessions left",
        )
    }

    /**
     * Should the radio screen warn that the trial is running out?
     *
     * Pure so it can be tested without Android: the real inputs
     * ([sessionsRemaining], [Entitlements.isUnlockedCached]) are both
     * environment-dependent, and the second is unconditionally true in debug
     * builds — testing through them would assert nothing.
     *
     * Warns only on the last two sessions. Counting down from five would be
     * nagging a user who has barely started; saying nothing at all is how the
     * paywall ends up arriving with no warning, which is the edge this closes.
     * Never warns an entitled user — they have nothing left to run out of.
     */
    fun warnThreshold(remaining: Int, entitled: Boolean): Boolean =
        !entitled && remaining in 1..2

    /** [warnThreshold] applied to the live environment. */
    fun shouldWarn(context: Context): Boolean =
        warnThreshold(sessionsRemaining(context), Entitlements.isUnlockedCached(context))

    /**
     * True when the trial is spent AND unpurchased — i.e. the user is at the
     * paywall *because they used it up*, not because they arrived locked. Lets
     * the gate say which of those happened instead of showing one blank wall
     * for both.
     */
    fun trialExhausted(context: Context): Boolean =
        !Entitlements.isUnlockedCached(context) && qualifyingSessions(context) >= FREE_SESSIONS

    /** Test/support hook: restore a full trial. */
    fun reset(context: Context) {
        prefs(context).edit().remove(KEY_COUNTED_SESSIONS).apply()
        Log.w(TAG, "Trial reset — $FREE_SESSIONS free sessions restored")
    }
}
