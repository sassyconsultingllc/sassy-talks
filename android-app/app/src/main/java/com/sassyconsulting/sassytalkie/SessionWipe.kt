// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import android.content.Context
import android.util.Log

/**
 * In-app + managed-restriction session wipe. Clears keys and control plane.
 * Silent enterprise wipe of the whole device still requires a DPC / EMM.
 */
object SessionWipe {
    private const val TAG = "SessionWipe"

    fun wipe(context: Context, source: String, audit: EmergencyAuditStore? = null) {
        try {
            audit?.append("wipe", "source=$source")
        } catch (_: Throwable) {}
        try {
            AuthenticatedControlPlane.clear()
        } catch (_: Throwable) {}
        try {
            SassyTalkNative.clearSession()
        } catch (t: Throwable) {
            Log.w(TAG, "native clearSession: ${t.message}")
        }
        try {
            context.getSharedPreferences(ManagedConfig.PREFS, Context.MODE_PRIVATE)
                .edit()
                .remove("live_translation_enabled")
                .apply()
        } catch (_: Throwable) {}
        Log.i(TAG, "Session wiped ($source). MDM/EMM is required for device-level wipe.")
    }

    fun applyManagedWipeIfRequested(context: Context, audit: EmergencyAuditStore? = null) {
        if (!ManagedConfig.forceSessionWipe(context)) return
        wipe(context, "managed_restriction", audit)
    }
}
