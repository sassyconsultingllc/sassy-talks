// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import android.content.pm.ServiceInfo
import android.os.Build

/** Foreground-service type mask for [WalkieService], shared with JVM tests. */
object ForegroundTypes {
    /**
     * On API 34+ the MICROPHONE type needs RECORD_AUDIO granted at the moment
     * of startForeground, or it throws SecurityException. On first launch the
     * permission dialog is still up when the service starts, so ask for the
     * other types now and re-promote with MICROPHONE after the grant.
     * CONNECTED_DEVICE is backed by CHANGE_WIFI_MULTICAST_STATE (install-time).
     */
    fun mask(sdk: Int, micGranted: Boolean): Int {
        var types = ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK or
            ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
        if (sdk >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE && micGranted) {
            types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        }
        return types
    }
}
