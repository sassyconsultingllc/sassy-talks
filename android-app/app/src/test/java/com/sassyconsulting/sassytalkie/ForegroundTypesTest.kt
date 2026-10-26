// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie

import android.content.pm.ServiceInfo
import org.junit.Assert.assertEquals
import org.junit.Test

class ForegroundTypesTest {
    private val base = ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK or
        ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE

    @Test
    fun `first launch on Android 14 promotes without the microphone type`() {
        assertEquals(base, ForegroundTypes.mask(sdk = 34, micGranted = false))
    }

    @Test
    fun `microphone type is added once RECORD_AUDIO is granted`() {
        assertEquals(
            base or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE,
            ForegroundTypes.mask(sdk = 34, micGranted = true),
        )
    }

    @Test
    fun `before Android 14 the microphone type is never requested`() {
        assertEquals(base, ForegroundTypes.mask(sdk = 33, micGranted = true))
        assertEquals(base, ForegroundTypes.mask(sdk = 29, micGranted = true))
    }
}
