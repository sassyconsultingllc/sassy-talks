// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

/**
 * Honest FIPS provider probe. This APK does not ship a CMVP-validated module.
 * If Conscrypt happens to be on the classpath it can be selected; that is still
 * not a FIPS 140 certificate claim.
 */
object FipsProvider {
    const val STATUS_NOT_PRESENT = "not_present"
    const val STATUS_CONSCRYPT_PRESENT = "conscrypt_present_not_cmvp"

    fun conscryptAvailable(): Boolean = try {
        Class.forName("org.conscrypt.Conscrypt")
        true
    } catch (_: Throwable) {
        false
    }

    fun status(): String =
        if (conscryptAvailable()) STATUS_CONSCRYPT_PRESENT else STATUS_NOT_PRESENT

    /** When MDM requires a FIPS module and none is present, TX must fail closed. */
    fun txAllowed(requireFips: Boolean): Boolean = !requireFips || conscryptAvailable()

    const val ABOUT_STATUS = "Not FIPS-validated / not CJIS-certified"

    const val ABOUT_DETAIL =
        "This product is not FIPS 140 validated and is not CJIS certified. " +
            "There is no CMVP-validated cryptographic module in this APK. " +
            "require_fips_provider fail-closes TX when no FIPS-capable provider is present."
}
