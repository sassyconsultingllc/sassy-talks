// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

/**
 * Decides what to do with `shared_prefs/sassy_session.xml` before it is opened
 * as EncryptedSharedPreferences.
 *
 * Old builds fell back to plain SharedPreferences when the Keystore failed and
 * wrote session keys into this file in the clear. Those must be removed. A
 * file EncryptedSharedPreferences owns must be left alone: it is recognised by
 * the Tink keyset entries it stores alongside the encrypted ones.
 *
 * The previous check looked for the keyset only in the first 256 characters.
 * SharedPreferences writes entries in hash order, so on many installs an
 * encrypted session entry (several hundred characters) came first, the file
 * was taken for plaintext, and every saved session and the cohort history
 * were deleted on each cold start.
 */
internal object SessionPrefsHygiene {

    /** Name prefix of the two keyset entries EncryptedSharedPreferences writes. */
    const val KEYSET_MARKER = "__androidx_security_crypto_encrypted_prefs_"

    /**
     * Entry names old builds wrote in the clear. EncryptedSharedPreferences
     * stores every name encrypted, so these never appear literally in a file
     * it wrote.
     */
    private val LEGACY_PLAINTEXT_NAME =
        Regex("""\sname="(session_ch_\d+|session_json|cohort_history_v\d+)"""")

    sealed interface Verdict {
        /** Encrypted store with nothing left over. */
        object Keep : Verdict

        /** No keyset at all: nothing here can be decrypted, and it may hold keys in the clear. */
        object WipeFile : Verdict

        /** Encrypted store that still carries entries an old build wrote in the clear. */
        data class RemoveEntries(val names: List<String>) : Verdict
    }

    fun inspect(xml: String): Verdict {
        if (!xml.contains(KEYSET_MARKER)) return Verdict.WipeFile
        val legacy = LEGACY_PLAINTEXT_NAME.findAll(xml).map { it.groupValues[1] }.distinct().toList()
        return if (legacy.isEmpty()) Verdict.Keep else Verdict.RemoveEntries(legacy)
    }
}
