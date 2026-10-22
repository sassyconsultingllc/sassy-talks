// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie.license

/**
 * Where the Play flavor's `refresh()` gets its answer.
 *
 * A stored credential of kind "promo" says only how the user got in first.
 * Someone whose promo lapsed and who then bought on Play still has
 * `credential_kind = "promo"`. The old refresh() returned false for them
 * without asking Play, so a paying user was put back behind the gate on every
 * launch, Restore said "No purchase found", and Buy answered ITEM_ALREADY_OWNED.
 * This is the Android twin of the iOS bug fixed in [Unreleased].
 *
 * Kept pure so the routing can be tested without Play or SharedPreferences.
 */
object PlayRefreshPolicy {

    /** Ask the relay first only while a promo receipt is still valid. */
    fun startWithPromo(kind: String?, promoReceiptValid: Boolean): Boolean =
        kind == "promo" && promoReceiptValid

    /** After a promo refresh, anything short of a valid promo falls through to Play. */
    fun askPlayAfterPromo(promoRefreshOk: Boolean, promoReceiptStillValid: Boolean): Boolean =
        !(promoRefreshOk || promoReceiptStillValid)
}
