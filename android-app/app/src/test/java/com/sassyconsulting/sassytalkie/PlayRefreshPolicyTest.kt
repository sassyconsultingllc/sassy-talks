// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import com.sassyconsulting.sassytalkie.license.PlayRefreshPolicy
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A user who came in on a promo and later bought on Play must be checked
 * against Play once the promo is no longer valid (see [PlayRefreshPolicy]).
 */
class PlayRefreshPolicyTest {

    @Test
    fun `lapsed promo is checked against Play`() {
        // The old refresh() answered false here without asking Play.
        assertFalse(PlayRefreshPolicy.startWithPromo(kind = "promo", promoReceiptValid = false))
    }

    @Test
    fun `valid promo asks the relay first`() {
        assertTrue(PlayRefreshPolicy.startWithPromo(kind = "promo", promoReceiptValid = true))
    }

    @Test
    fun `no promo goes straight to Play`() {
        assertFalse(PlayRefreshPolicy.startWithPromo(kind = null, promoReceiptValid = false))
        assertFalse(PlayRefreshPolicy.startWithPromo(kind = "license", promoReceiptValid = true))
    }

    @Test
    fun `promo rejected on refresh falls through to Play`() {
        // Revoked or capped promo: the user may still own the Play purchase.
        assertTrue(PlayRefreshPolicy.askPlayAfterPromo(promoRefreshOk = false, promoReceiptStillValid = false))
    }

    @Test
    fun `promo still good does not ask Play`() {
        assertFalse(PlayRefreshPolicy.askPlayAfterPromo(promoRefreshOk = true, promoReceiptStillValid = true))
        // Network error during refresh keeps an unexpired receipt.
        assertFalse(PlayRefreshPolicy.askPlayAfterPromo(promoRefreshOk = false, promoReceiptStillValid = true))
    }
}
