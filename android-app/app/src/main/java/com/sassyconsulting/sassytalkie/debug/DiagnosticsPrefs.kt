// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-4CO25RYXFSBH
package com.sassyconsulting.sassytalkie.debug

import android.content.Context
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Runtime toggle for the on-screen diagnostics overlay.
 *
 * Unlike `BuildConfig.DEBUG`, this is honoured in RELEASE builds — the overlay
 * is meant for on-the-go field testing of shipped APKs. Default OFF so normal
 * users never see it; the user flips it from Settings → Diagnostics. The choice
 * is persisted so it survives process death during a test session.
 */
object DiagnosticsPrefs {
    private const val PREFS = "diagnostics_prefs"
    private const val KEY_OVERLAY = "overlay_enabled"

    private val _overlayEnabled = MutableStateFlow(false)
    val overlayEnabled: StateFlow<Boolean> = _overlayEnabled.asStateFlow()

    private var appContext: Context? = null

    /** Call once from Application/Activity onCreate before the UI reads the flow. */
    fun init(context: Context) {
        val ctx = context.applicationContext
        appContext = ctx
        _overlayEnabled.value = ctx
            .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getBoolean(KEY_OVERLAY, false)
    }

    fun setOverlayEnabled(enabled: Boolean) {
        val allowed = appContext?.let {
            com.sassyconsulting.sassytalkie.ManagedConfig.diagnosticsAllowed(it)
        } ?: true
        val next = enabled && allowed
        _overlayEnabled.value = next
        appContext
            ?.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            ?.edit()
            ?.putBoolean(KEY_OVERLAY, next)
            ?.apply()
    }

    /**
     * Overlay is debug/diag only. Release builds show it solely when the
     * operator enables Diagnostics. Never on Auth/Profile/Gate chrome.
     * MDM can force it off via enable_diagnostics_overlay=false.
     */
    fun shouldShowOverlay(
        debugBuild: Boolean,
        diagEnabled: Boolean,
        radioUiVisible: Boolean,
        managedAllowed: Boolean = true,
    ): Boolean = managedAllowed && radioUiVisible && (debugBuild || diagEnabled)
}
