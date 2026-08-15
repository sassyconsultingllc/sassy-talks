// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

/**
 * API 34+ forbids starting a microphone foreground service from an FCM
 * receiver. Cold wake must be a user-visible notification the operator taps;
 * a warm WalkieService may receive ACTION_WAKE without promoting FGS+mic.
 *
 * Remaining OEM limits (not solvable in-app):
 * - Xiaomi/HyperOS, Huawei, Oppo/ColorOS, Samsung: FCM data-only may be
 *   delayed or dropped unless autostart / battery-unrestricted is granted.
 * - Force-stop from Recents on some OEMs prevents FCM until the user opens
 *   the app again.
 * - Notification permission denied on API 33+ means the cold-wake bootstrap
 *   cannot surface; the user must open the app.
 * - Background activity starts from FCM are blocked on API 29+; this policy
 *   never calls startActivity from the receiver.
 */
internal object FcmWakePolicy {
    const val WAKE_CHANNEL_ID = "sassytalkie_fcm_wake"
    const val WAKE_NOTIFICATION_ID = 42

    enum class Path { WarmService, VisibleNotification }

    fun path(serviceRunning: Boolean): Path =
        if (serviceRunning) Path.WarmService else Path.VisibleNotification

    fun wakeChannelImportance(): Int = android.app.NotificationManager.IMPORTANCE_HIGH

    fun coldWakeUsesPersistentHighPriorityNotification(): Boolean = true

    /** Illegal on API 34+ from an FCM/background start. */
    fun mayStartMicrophoneFgsFromFcm(sdkInt: Int): Boolean = sdkInt < 34

    fun mayStartActivityFromFcm(): Boolean = false
}
