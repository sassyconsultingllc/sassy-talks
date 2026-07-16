// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-A2TKP2V4BDOC
package com.sassyconsulting.sassytalkie

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class CohortHistoryInstrumentedTest {

    @Before
    fun setUp() {
        val ctx = ApplicationProvider.getApplicationContext<Context>()
        SassyTalkNative.appContext = ctx
        SassyTalkNative.init()
        SassyTalkNative.clearCohortHistory()
        SassyTalkNative.clearSession()
    }

    @Test
    fun host_generate_creates_hosted_record() {
        val qr = SassyTalkNative.generateChannelQR(1, 24, "Math 101 P1")
        assertTrue(qr.isNotEmpty())
        val history = JSONArray(SassyTalkNative.getCohortHistory())
        assertEquals(1, history.length())
        val rec = history.getJSONObject(0)
        assertEquals("Math 101 P1", rec.getString("group_name"))
        assertEquals(1, rec.getInt("channel"))
        assertEquals("Hosted", rec.getString("role"))
    }

    @Test
    fun clear_session_preserves_cohort_history() {
        SassyTalkNative.generateChannelQR(1, 24, "Math 101 P1")
        SassyTalkNative.clearSession()
        assertFalse(SassyTalkNative.isAuthenticated())
        val history = JSONArray(SassyTalkNative.getCohortHistory())
        assertEquals(1, history.length())
    }

    @Test
    fun import_legacy_qr_mints_cohort_id_as_joined() {
        val now = System.currentTimeMillis() / 1000
        val keyBytes = ByteArray(32) { (it + 1).toByte() }
        val keyB64 = android.util.Base64.encodeToString(keyBytes, android.util.Base64.NO_WRAP)
        val legacy = JSONObject().apply {
            put("key", keyB64)
            put("device", "Legacy Host")
            put("created_at", now)
            put("expires_at", now + 3600)
            put("session_id", "legacy-sid-uuid")
            put("channel", 2)
            put("group_name", "OldGroup")
            // intentionally no cohort_id
        }.toString()

        val ok = SassyTalkNative.importSessionFromQR(legacy)
        assertTrue(ok)

        val history = JSONArray(SassyTalkNative.getCohortHistory())
        assertEquals(1, history.length().toLong())
        val rec = history.getJSONObject(0)
        assertEquals("Joined", rec.getString("role"))
        assertEquals("Legacy Host", rec.getString("host_device"))
        val cid = rec.getString("cohort_id")
        assertTrue("minted cohort_id must be non-empty", cid.isNotEmpty())
        // throws IllegalArgumentException on malformed UUID, failing the test
        java.util.UUID.fromString(cid)
    }

    @Test
    fun rejoin_hosted_keeps_cohort_id_changes_session_id() {
        val qr1 = SassyTalkNative.generateChannelQR(1, 24, "Math 101 P1")
        val cid1 = JSONObject(qr1).getString("cohort_id")
        val sid1 = JSONObject(qr1).getString("session_id")
        SassyTalkNative.clearSession()

        val qr2 = SassyTalkNative.generateChannelQR(1, 24, "Math 101 P1", cid1)
        val cid2 = JSONObject(qr2).getString("cohort_id")
        val sid2 = JSONObject(qr2).getString("session_id")

        assertEquals("cohort_id must persist across rejoin", cid1, cid2)
        assertNotEquals("session_id must rotate on rejoin", sid1, sid2)

        val history = JSONArray(SassyTalkNative.getCohortHistory())
        assertEquals("rejoin must not create a duplicate record", 1, history.length().toLong())
    }
}
