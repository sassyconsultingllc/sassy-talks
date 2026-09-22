// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings

/**
 * OEM battery / autostart deep-links. FCM data-only wakes are delayed or
 * dropped on several OEM skins unless the user grants autostart / unrestricted
 * battery. These intents are best-effort; [appDetailsIntent] always exists.
 */
object OemBatteryGuidance {
    fun manufacturer(): String = Build.MANUFACTURER.orEmpty()

    fun summary(manufacturer: String = manufacturer()): String {
        val m = manufacturer.lowercase()
        return when {
            m.contains("xiaomi") || m.contains("redmi") || m.contains("poco") ->
                "Xiaomi/HyperOS may delay FCM unless Autostart and unrestricted battery are granted."
            m.contains("samsung") ->
                "Samsung may put the app to sleep. Disable Sleeping apps / Deep sleeping for SassyTalk."
            m.contains("huawei") || m.contains("honor") ->
                "Huawei/Honor may block FCM unless App launch / ignore battery optimizations is allowed."
            m.contains("oppo") || m.contains("realme") || m.contains("oneplus") ->
                "ColorOS/OxygenOS may delay FCM unless Autostart and battery unrestricted are granted."
            else ->
                "Some phones delay background wake-ups unless battery optimizations are ignored for this app."
        }
    }

    fun appDetailsIntent(context: Context): Intent =
        Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).apply {
            data = Uri.fromParts("package", context.packageName, null)
        }

    fun ignoreBatteryIntent(context: Context): Intent =
        Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS).apply {
            putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
        }

    fun primarySettingsIntent(context: Context): Intent {
        val m = manufacturer().lowercase()
        val candidates = ArrayList<Intent>()
        when {
            m.contains("xiaomi") || m.contains("redmi") || m.contains("poco") -> {
                candidates += Intent("miui.intent.action.OP_AUTO_START")
                candidates += Intent().setClassName(
                    "com.miui.securitycenter",
                    "com.miui.permcenter.autostart.AutoStartManagementActivity",
                )
            }
            m.contains("samsung") -> {
                candidates += Intent().setClassName(
                    "com.samsung.android.lool",
                    "com.samsung.android.sm.ui.battery.BatteryActivity",
                )
                candidates += Intent("com.samsung.android.sm.ACTION_BATTERY")
            }
            m.contains("huawei") || m.contains("honor") -> {
                candidates += Intent().setClassName(
                    "com.huawei.systemmanager",
                    "com.huawei.systemmanager.startupmgr.ui.StartupNormalAppListActivity",
                )
            }
            m.contains("oppo") || m.contains("realme") -> {
                candidates += Intent().setClassName(
                    "com.coloros.safecenter",
                    "com.coloros.safecenter.startupapp.StartupAppListActivity",
                )
            }
            m.contains("oneplus") -> {
                candidates += Intent().setClassName(
                    "com.oneplus.security",
                    "com.oneplus.security.chainlaunch.view.ChainLaunchAppListActivity",
                )
            }
        }
        candidates += ignoreBatteryIntent(context)
        candidates += appDetailsIntent(context)
        val pm = context.packageManager
        return candidates.firstOrNull { it.resolveActivity(pm) != null } ?: appDetailsIntent(context)
    }
}
