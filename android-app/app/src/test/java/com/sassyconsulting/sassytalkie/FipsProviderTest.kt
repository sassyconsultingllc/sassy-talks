// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class FipsProviderTest {
    @Test
    fun `tx is allowed when FIPS is not required`() {
        assertTrue(FipsProvider.txAllowed(false))
    }

    @Test
    fun `require FIPS fail-closes when Conscrypt is absent`() {
        if (FipsProvider.conscryptAvailable()) {
            assertTrue(FipsProvider.txAllowed(true))
        } else {
            assertFalse(FipsProvider.txAllowed(true))
            assertTrue(FipsProvider.status() == FipsProvider.STATUS_NOT_PRESENT)
        }
    }

    @Test
    fun `about status is not a certification claim`() {
        assertTrue(FipsProvider.ABOUT_STATUS.contains("Not FIPS-validated"))
        assertTrue(FipsProvider.ABOUT_STATUS.contains("not CJIS-certified"))
        assertFalse(FipsProvider.ABOUT_STATUS.contains("Certified"))
        assertTrue(FipsProvider.ABOUT_DETAIL.contains("not FIPS 140 validated"))
        assertTrue(FipsProvider.txAllowed(false))
    }
}
