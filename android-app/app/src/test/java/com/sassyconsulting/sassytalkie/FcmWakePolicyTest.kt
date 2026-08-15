// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie

import android.app.NotificationManager
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class FcmWakePolicyTest {
    @Test
    fun `cold start uses a visible notification not mic FGS`() {
        assertEquals(FcmWakePolicy.Path.VisibleNotification, FcmWakePolicy.path(false))
        assertEquals(FcmWakePolicy.Path.WarmService, FcmWakePolicy.path(true))
        assertTrue(FcmWakePolicy.coldWakeUsesPersistentHighPriorityNotification())
        assertEquals(NotificationManager.IMPORTANCE_HIGH, FcmWakePolicy.wakeChannelImportance())
    }

    @Test
    fun `api 34 plus cannot start microphone FGS from FCM`() {
        assertTrue(FcmWakePolicy.mayStartMicrophoneFgsFromFcm(33))
        assertFalse(FcmWakePolicy.mayStartMicrophoneFgsFromFcm(34))
        assertFalse(FcmWakePolicy.mayStartMicrophoneFgsFromFcm(36))
        assertFalse(FcmWakePolicy.mayStartActivityFromFcm())
    }

    @Test
    fun `oem battery guidance always has an app-details fallback`() {
        assertTrue(OemBatteryGuidance.summary("Xiaomi").contains("FCM") || OemBatteryGuidance.summary("Xiaomi").contains("Autostart"))
        assertTrue(OemBatteryGuidance.summary("samsung").contains("Samsung"))
        assertTrue(OemBatteryGuidance.summary("Google").isNotBlank())
    }
}
