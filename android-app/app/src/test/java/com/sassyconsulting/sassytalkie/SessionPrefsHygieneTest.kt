// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class SessionPrefsHygieneTest {

    private val header = "<?xml version='1.0' encoding='utf-8' standalone='yes' ?>\n<map>\n"
    private val footer = "</map>\n"

    /** An encrypted entry as EncryptedSharedPreferences writes it: base64 name and value. */
    private fun encryptedEntry(seed: Char) =
        "    <string name=\"AT${seed.toString().repeat(60)}==\">AV${seed.toString().repeat(500)}=</string>\n"

    private val keysets =
        "    <string name=\"__androidx_security_crypto_encrypted_prefs_key_keyset__\">12a901" +
            "0".repeat(300) + "</string>\n" +
            "    <string name=\"__androidx_security_crypto_encrypted_prefs_value_keyset__\">128801" +
            "0".repeat(300) + "</string>\n"

    /**
     * Regression: the keysets come after an encrypted session entry, so they
     * are nowhere in the first 256 characters. This file used to be wiped on
     * every cold start, taking the saved session with it.
     */
    @Test
    fun encryptedFileWithTheKeysetLateIsKept() {
        val xml = header + encryptedEntry('a') + encryptedEntry('b') + keysets + footer
        assertTrue(xml.indexOf(SessionPrefsHygiene.KEYSET_MARKER) > 256)
        assertEquals(SessionPrefsHygiene.Verdict.Keep, SessionPrefsHygiene.inspect(xml))
    }

    @Test
    fun encryptedFileWithTheKeysetFirstIsKept() {
        val xml = header + keysets + encryptedEntry('a') + footer
        assertEquals(SessionPrefsHygiene.Verdict.Keep, SessionPrefsHygiene.inspect(xml))
    }

    @Test
    fun plaintextFileFromAnOldBuildIsWiped() {
        val xml = header +
            "    <string name=\"session_ch_1\">{&quot;key&quot;:&quot;c2VjcmV0&quot;}</string>\n" +
            "    <string name=\"session_json\">{}</string>\n" + footer
        assertEquals(SessionPrefsHygiene.Verdict.WipeFile, SessionPrefsHygiene.inspect(xml))
    }

    @Test
    fun emptyFileWithoutAKeysetIsWiped() {
        assertEquals(SessionPrefsHygiene.Verdict.WipeFile, SessionPrefsHygiene.inspect("$header$footer"))
        assertEquals(SessionPrefsHygiene.Verdict.WipeFile, SessionPrefsHygiene.inspect(""))
    }

    @Test
    fun plaintextLeftoversInAnEncryptedFileAreRemovedOneByOne() {
        val xml = header + encryptedEntry('a') +
            "    <string name=\"session_ch_3\">{}</string>\n" +
            keysets +
            "    <string name=\"session_json\">{}</string>\n" + footer
        assertEquals(
            SessionPrefsHygiene.Verdict.RemoveEntries(listOf("session_ch_3", "session_json")),
            SessionPrefsHygiene.inspect(xml),
        )
    }

    @Test
    fun aLegacyNameInsideAnEncryptedValueIsNotMistakenForAnEntry() {
        // Only a real XML attribute counts, not the same text inside a value.
        val xml = header + keysets +
            "    <string name=\"ATxyz==\">name=\"session_ch_1\"</string>\n" + footer
        // The value text is preceded by '>', not whitespace, so it does not match.
        assertEquals(SessionPrefsHygiene.Verdict.Keep, SessionPrefsHygiene.inspect(xml))
    }
}
