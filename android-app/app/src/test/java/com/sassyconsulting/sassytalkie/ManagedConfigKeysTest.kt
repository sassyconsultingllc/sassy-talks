// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ManagedConfigKeysTest {
    @Test
    fun `restriction keys cover MDM surface`() {
        val keys = ManagedConfig.ALL_KEYS
        assertTrue(keys.contains(ManagedConfig.KEY_LOCK_SCREEN_PTT))
        assertTrue(keys.contains(ManagedConfig.KEY_REQUIRE_RELAY))
        assertTrue(keys.contains(ManagedConfig.KEY_ENABLE_NOTIFICATIONS))
        assertTrue(keys.contains(ManagedConfig.KEY_ENABLE_TRANSLATION))
        assertTrue(keys.contains(ManagedConfig.KEY_ENABLE_DIAGNOSTICS))
        assertTrue(keys.contains(ManagedConfig.KEY_MAX_TX_SECONDS))
        assertTrue(keys.contains(ManagedConfig.KEY_ENROLLMENT_TOKEN))
        assertTrue(keys.contains(ManagedConfig.KEY_FORCE_SESSION_WIPE))
        assertTrue(keys.contains(ManagedConfig.KEY_REQUIRE_FIPS))
        assertTrue(keys.contains(ManagedConfig.KEY_REQUIRE_TLS_PINNING))
        assertTrue(RelayTlsPins.productionDefaultEnabled)
        assertEquals(15, keys.size)
    }

    @Test
    fun `diagnostics overlay is allowed unless MDM denies it`() {
        assertTrue(ManagedConfig.DEFAULT_DIAGNOSTICS_ALLOWED)
    }

    @Test
    fun `lock-screen PTT actions require notifications`() {
        assertFalse(ManagedConfig.lockScreenPttActionsAllowed(true, false))
        assertFalse(ManagedConfig.lockScreenPttActionsAllowed(false, true))
        assertTrue(ManagedConfig.lockScreenPttActionsAllowed(true, true))
    }

    @Test
    fun `supervisor is the only privileged wipe role string`() {
        assertEquals(ManagedConfig.ROLE_OPERATOR, "operator")
        assertEquals(ManagedConfig.ROLE_SUPERVISOR, "supervisor")
    }
}
