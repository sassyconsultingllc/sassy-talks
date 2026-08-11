// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.sassyconsulting.sassytalkie.license.TrialGate
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Free-trial accounting. On-device because it exercises real
 * SharedPreferences, which is where the counter actually lives.
 */
@RunWith(AndroidJUnit4::class)
class TrialGateInstrumentedTest {

    private val ctx: Context get() = ApplicationProvider.getApplicationContext()

    @Before
    fun setUp() {
        TrialGate.reset(ctx)
    }

    @Test
    fun fresh_install_has_the_full_trial_and_no_paywall() {
        assertEquals(0, TrialGate.qualifyingSessions(ctx))
        assertEquals(TrialGate.FREE_SESSIONS, TrialGate.sessionsRemaining(ctx))
        assertTrue(TrialGate.inTrial(ctx))
        // NOTE: mayUseRadio is unconditionally true in a debug build because
        // Entitlements bypasses for BuildConfig.DEBUG, so it proves nothing
        // here. Assert the trial state itself, which is what governs a release.
        assertTrue("fresh install must be in trial", TrialGate.inTrial(ctx))
    }

    /**
     * The rule that matters. A session is only counted when a peer was present,
     * and the same room joined repeatedly is ONE session — otherwise a user who
     * rejoins their own channel through the day would hit the paywall without
     * ever having talked to a new group.
     */
    @Test
    fun the_same_session_counts_once_no_matter_how_often_it_is_rejoined() {
        repeat(10) { TrialGate.noteQualifyingSession(ctx, "session-alpha") }
        assertEquals(1, TrialGate.qualifyingSessions(ctx))
        assertEquals(TrialGate.FREE_SESSIONS - 1, TrialGate.sessionsRemaining(ctx))
    }

    @Test
    fun distinct_sessions_each_count_once() {
        listOf("a", "b", "c").forEach { TrialGate.noteQualifyingSession(ctx, it) }
        assertEquals(3, TrialGate.qualifyingSessions(ctx))
    }

    /**
     * A session with no peer never reaches noteQualifyingSession, so hosting a
     * QR nobody scans costs nothing. Modelled here as the absence of the call.
     */
    @Test
    fun a_solo_session_does_not_consume_the_trial() {
        assertEquals(0, TrialGate.qualifyingSessions(ctx))
        assertTrue(TrialGate.inTrial(ctx))
    }

    @Test
    fun blank_session_ids_are_ignored() {
        TrialGate.noteQualifyingSession(ctx, null)
        TrialGate.noteQualifyingSession(ctx, "")
        TrialGate.noteQualifyingSession(ctx, "   ")
        assertEquals(0, TrialGate.qualifyingSessions(ctx))
    }

    @Test
    fun paywall_appears_only_after_five_qualifying_sessions() {
        for (i in 1..TrialGate.FREE_SESSIONS) {
            assertTrue("still in trial before session $i", TrialGate.inTrial(ctx))
            TrialGate.noteQualifyingSession(ctx, "session-$i")
        }
        assertEquals(TrialGate.FREE_SESSIONS, TrialGate.qualifyingSessions(ctx))
        assertEquals(0, TrialGate.sessionsRemaining(ctx))
        assertFalse("trial is spent after 5", TrialGate.inTrial(ctx))
    }

    /** Counting must never run away past the threshold. */
    @Test
    fun remaining_never_goes_negative() {
        repeat(TrialGate.FREE_SESSIONS + 7) { TrialGate.noteQualifyingSession(ctx, "s$it") }
        assertEquals(0, TrialGate.sessionsRemaining(ctx))
    }
}
