// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import android.app.admin.DevicePolicyManager
import android.content.Context
import android.os.UserManager

/**
 * Detects work-profile / device-owner state for logging and restriction honor.
 * Does **not** block managed profiles — agencies deploy this app through MDM.
 */
object ProfileChecker {
    enum class ProfileKind { PERSONAL, WORK_PROFILE, MANAGED_DEVICE, UNKNOWN }

    fun profileKind(context: Context): ProfileKind {
        val um = try {
            context.getSystemService(Context.USER_SERVICE) as? UserManager
        } catch (_: Throwable) {
            null
        }
        val dpm = try {
            context.getSystemService(Context.DEVICE_POLICY_SERVICE) as? DevicePolicyManager
        } catch (_: Throwable) {
            null
        }
        val managedProfile = try {
            um?.isManagedProfile == true
        } catch (_: Throwable) {
            false
        }
        val deviceOwner = try {
            dpm?.isDeviceOwnerApp(context.packageName) == true ||
                (android.os.Build.VERSION.SDK_INT >= 18 && dpm?.isProfileOwnerApp(context.packageName) == true)
        } catch (_: Throwable) {
            false
        }
        return when {
            deviceOwner && !managedProfile -> ProfileKind.MANAGED_DEVICE
            managedProfile -> ProfileKind.WORK_PROFILE
            um != null -> ProfileKind.PERSONAL
            else -> ProfileKind.UNKNOWN
        }
    }

    /** Always false: blocking work profiles would break agency MDM deploy. */
    fun shouldBlock(context: Context): Boolean {
        profileKind(context)
        return false
    }

    fun hasManagedRestrictions(context: Context): Boolean {
        val bundle = ManagedConfig.restrictions(context) ?: return false
        return ManagedConfig.ALL_KEYS.any { bundle.containsKey(it) }
    }
}
