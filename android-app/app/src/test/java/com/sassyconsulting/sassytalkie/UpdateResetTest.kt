// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Trigger logic for the post-update reset.
 *
 * The decision is a pure function precisely so it can be pinned here: getting
 * it wrong is not a crash, it is either a reset that never fires (the bug this
 * exists to fix, back again and invisible) or one that fires on every launch
 * (clearing the audio cache and timeline constantly, which looks like data
 * loss to the user).
 */
class UpdateResetTest {

    @Test
    fun `same version is not a transition`() {
        assertFalse(UpdateReset.isVersionTransition(last = 73, current = 73))
    }

    @Test
    fun `upgrade is a transition`() {
        assertTrue(UpdateReset.isVersionTransition(last = 72, current = 73))
    }

    /**
     * A downgrade leaves state written by a NEWER binary, which is at least as
     * suspect as stale state — Play can serve an older build after a halted
     * rollout, and sideloading over a newer build is routine in testing.
     */
    @Test
    fun `downgrade is a transition`() {
        assertTrue(UpdateReset.isVersionTransition(last = 74, current = 73))
    }

    /**
     * First install must NOT reset. There is nothing stale to clear, and
     * firing here would add pointless work to the coldest start the app ever
     * has — the one where the user is waiting on the profile screen.
     */
    @Test
    fun `first install is not a transition`() {
        assertFalse(UpdateReset.isVersionTransition(last = 0, current = 73))
    }

    @Test
    fun `first install of any version is not a transition`() {
        for (v in listOf(1, 68, 73, 999)) {
            assertFalse("v$v", UpdateReset.isVersionTransition(last = 0, current = v))
        }
    }

    /** Full-reset is opt-in: the default must never wipe session keys. */
    @Test
    fun `full reset key is distinct from the version marker`() {
        assertTrue(UpdateReset.KEY_FULL_RESET != UpdateReset.KEY_LAST_VERSION)
    }
}
