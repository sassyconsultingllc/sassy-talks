// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie.license

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LicensePromoTest {
    @Test
    fun `revoked, unknown or expired codes are revocations`() {
        assertTrue(LicensePromo.isRevocation(404, "Unknown promo code"))
        assertTrue(LicensePromo.isRevocation(403, "Promo code revoked"))
        assertTrue(LicensePromo.isRevocation(403, "Promo code expired"))
    }

    @Test
    fun `rate limits and transient errors keep the promo`() {
        assertFalse(LicensePromo.isRevocation(429, "Too many attempts; try again later"))
        assertFalse(LicensePromo.isRevocation(408, ""))
        assertFalse(LicensePromo.isRevocation(401, "Unauthorized"))
        assertFalse(LicensePromo.isRevocation(400, "Malformed JSON body"))
    }

    /** A changed device id reads as "fully redeemed"; the code itself is still good. */
    @Test
    fun `fully redeemed on refresh is not a revocation`() {
        assertFalse(LicensePromo.isRevocation(403, "Promo code fully redeemed"))
    }
}
