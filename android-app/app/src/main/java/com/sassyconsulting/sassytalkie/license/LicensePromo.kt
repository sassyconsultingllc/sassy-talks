// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-Q7BFTEJJ46J2
package com.sassyconsulting.sassytalkie.license

import android.content.Context
import android.util.Log
import com.sassyconsulting.sassytalkie.BuildConfig
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import java.util.concurrent.TimeUnit

/**
 * Promo-code redemption against the relay worker's `/license/promo` endpoint.
 * Used by the Play paywall (friends & family bypass) and refresh logic.
 */
object LicensePromo {
    private const val TAG = "LicensePromo"
    private const val REFRESH_WHEN_REMAINING_SEC = 15L * 24 * 3600
    private val LICENSE_RE = Regex("^SASSY(?:-[A-HJ-NP-Z2-9]{5}){4}$")
    private val PROMO_RE = Regex("^[A-Z0-9-]{6,40}$")

    sealed class RedeemResult {
        data object Ok : RedeemResult()
        data object InvalidFormat : RedeemResult()
        data object NetworkError : RedeemResult()
        data class Rejected(val message: String) : RedeemResult()
    }

    fun hasValidReceipt(context: Context): Boolean {
        val p = LicenseStore.prefs(context) ?: return false
        if (p.getString(LicenseStore.KEY_KIND, null) != "promo") return false
        val exp = p.getLong(LicenseStore.KEY_RECEIPT_EXP, 0L)
        return exp > System.currentTimeMillis() / 1000
    }

    /** Mirrors worker [normalizePromo]: 6–40 chars A-Z 0-9 -, not a license key. */
    fun normalizeCode(raw: String): String? {
        val code = raw.trim().uppercase().replace(Regex("\\s+"), "")
        if (LICENSE_RE.matches(code)) return null
        return if (PROMO_RE.matches(code)) code else null
    }

    fun redeemBlocking(context: Context, rawCode: String): RedeemResult {
        val code = normalizeCode(rawCode) ?: return RedeemResult.InvalidFormat
        return when (val api = callPromo(context, code)) {
            is ApiResult.Ok -> {
                persist(context, code, api.expiresAt)
                RedeemResult.Ok
            }
            is ApiResult.Rejected -> RedeemResult.Rejected(api.message)
            ApiResult.NetworkError -> RedeemResult.NetworkError
        }
    }

    fun refreshIfNeeded(context: Context, onResult: (Boolean) -> Unit) {
        val p = LicenseStore.prefs(context) ?: return onResult(false)
        val code = p.getString(LicenseStore.KEY_LICENSE, null) ?: return onResult(false)
        if (p.getString(LicenseStore.KEY_KIND, null) != "promo") return onResult(false)
        val exp = p.getLong(LicenseStore.KEY_RECEIPT_EXP, 0L)
        val now = System.currentTimeMillis() / 1000
        if (exp <= now) return onResult(false)
        if (exp - now > REFRESH_WHEN_REMAINING_SEC) return onResult(true)

        CoroutineScope(Dispatchers.IO).launch {
            when (val res = callPromo(context, code)) {
                is ApiResult.Ok -> {
                    p.edit().putLong(LicenseStore.KEY_RECEIPT_EXP, res.expiresAt).apply()
                    onResult(true)
                }
                is ApiResult.Rejected -> {
                    Log.w(TAG, "Promo rejected on refresh: ${res.message}")
                    p.edit()
                        .remove(LicenseStore.KEY_RECEIPT_EXP)
                        .remove(LicenseStore.KEY_KIND)
                        .remove(LicenseStore.KEY_LICENSE)
                        .putBoolean(LicenseStore.KEY_UNLOCKED, false)
                        .apply()
                    onResult(false)
                }
                ApiResult.NetworkError -> onResult(exp > now)
            }
        }
    }

    private fun persist(context: Context, code: String, expiresAt: Long) {
        // Promo entitlement is RECEIPT-DRIVEN (time-limited): store only the
        // expiry + kind, and let isUnlockedCached honor it via hasValidReceipt.
        // Never set the permanent KEY_UNLOCKED flag here — that flag is checked
        // first and would make a 30-day promo last forever.
        LicenseStore.prefs(context)?.edit()
            ?.putString(LicenseStore.KEY_LICENSE, code)
            ?.putString(LicenseStore.KEY_KIND, "promo")
            ?.putLong(LicenseStore.KEY_RECEIPT_EXP, expiresAt)
            ?.apply()
    }

    private sealed class ApiResult {
        class Ok(val expiresAt: Long) : ApiResult()
        class Rejected(val message: String) : ApiResult()
        data object NetworkError : ApiResult()
    }

    private val http = OkHttpClient.Builder()
        .connectTimeout(10, TimeUnit.SECONDS)
        .readTimeout(10, TimeUnit.SECONDS)
        .build()

    private fun callPromo(context: Context, code: String): ApiResult {
        return try {
            val body = JSONObject()
                .put("code", code)
                .put("device_id", deviceId(context))
                .put("device_name", android.os.Build.MODEL)
                .put("app_version", BuildConfig.VERSION_NAME)
                .toString()
                .toRequestBody("application/json".toMediaType())
            val req = Request.Builder()
                .url(BuildConfig.LICENSE_API_BASE + "/license/promo")
                .post(body)
                .build()
            http.newCall(req).execute().use { resp ->
                val json = JSONObject(resp.body?.string() ?: "{}")
                when {
                    resp.isSuccessful && json.optBoolean("ok") ->
                        ApiResult.Ok(json.optLong("expires_at"))
                    resp.code >= 500 -> ApiResult.NetworkError
                    else -> ApiResult.Rejected(json.optString("error", "Promo rejected"))
                }
            }
        } catch (e: Exception) {
            Log.w(TAG, "promo call failed: ${e.message}")
            ApiResult.NetworkError
        }
    }

    private fun deviceId(context: Context): String = LicenseStore.deviceId(context)
}
