package com.sassyconsulting.sassytalkie.license

import android.app.Activity
import android.content.Context
import android.util.Log
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.android.billingclient.api.AcknowledgePurchaseParams
import com.android.billingclient.api.BillingClient
import com.android.billingclient.api.BillingClientStateListener
import com.android.billingclient.api.BillingFlowParams
import com.android.billingclient.api.BillingResult
import com.android.billingclient.api.PendingPurchasesParams
import com.android.billingclient.api.ProductDetails
import com.android.billingclient.api.Purchase
import com.android.billingclient.api.QueryProductDetailsParams
import com.android.billingclient.api.QueryPurchasesParams
import com.sassyconsulting.sassytalkie.ui.theme.DarkBg
import com.sassyconsulting.sassytalkie.ui.theme.PrimaryBlue
import com.sassyconsulting.sassytalkie.ui.theme.StatusDisconnected
import com.sassyconsulting.sassytalkie.ui.theme.Teal
import com.sassyconsulting.sassytalkie.ui.theme.TextGray

/**
 * Play-flavor entitlement gate: one-time Google Play Billing purchase.
 *
 * Contract shared with the direct flavor (same fully-qualified name, different
 * source set — AppNavigation compiles against whichever flavor is built):
 *   - [isUnlockedCached]  fast, offline check for startup routing
 *   - [refresh]           silent background re-verification (restores the
 *                         purchase on reinstall, drops it after a refund)
 *   - [GateScreen]        full-screen paywall shown while locked
 */
object Entitlements {
    private const val TAG = "Entitlements"
    private const val PRODUCT_ID = "sassytalkie_unlock"

    fun isUnlockedCached(context: Context): Boolean =
        LicenseStore.prefs(context)?.getBoolean(LicenseStore.KEY_UNLOCKED, false) ?: false

    /**
     * Reconnect to Play and reconcile the cached entitlement with the store's
     * answer. Play's local purchase cache works offline, so an OK response is
     * authoritative in both directions: purchase found → cache unlock (covers
     * reinstall/second device), definitively absent → drop it (refund).
     * Connection failures leave the cache untouched and report the cached state.
     */
    fun refresh(context: Context, onResult: (Boolean) -> Unit = {}) {
        val client = BillingClient.newBuilder(context.applicationContext)
            .setListener { _, _ -> } // refresh never launches a flow
            .enablePendingPurchases(
                PendingPurchasesParams.newBuilder().enableOneTimeProducts().build(),
            )
            .build()
        client.startConnection(object : BillingClientStateListener {
            override fun onBillingSetupFinished(result: BillingResult) {
                if (result.responseCode != BillingClient.BillingResponseCode.OK) {
                    onResult(isUnlockedCached(context))
                    client.endConnection()
                    return
                }
                client.queryPurchasesAsync(
                    QueryPurchasesParams.newBuilder()
                        .setProductType(BillingClient.ProductType.INAPP).build(),
                ) { qr, purchases ->
                    if (qr.responseCode != BillingClient.BillingResponseCode.OK) {
                        onResult(isUnlockedCached(context))
                    } else {
                        val owned = purchases.any {
                            it.products.contains(PRODUCT_ID) &&
                                it.purchaseState == Purchase.PurchaseState.PURCHASED
                        }
                        if (owned) {
                            purchases.filter { it.products.contains(PRODUCT_ID) && !it.isAcknowledged }
                                .forEach { acknowledge(client, it) }
                            setUnlocked(context, true)
                        } else if (isUnlockedCached(context)) {
                            Log.w(TAG, "Play reports no purchase — revoking cached unlock")
                            setUnlocked(context, false)
                        }
                        onResult(owned)
                    }
                    client.endConnection()
                }
            }

            override fun onBillingServiceDisconnected() {}
        })
    }

    @Composable
    fun GateScreen(onUnlocked: () -> Unit) {
        val context = LocalContext.current
        var price by remember { mutableStateOf<String?>(null) }
        var details by remember { mutableStateOf<ProductDetails?>(null) }
        var error by remember { mutableStateOf<String?>(null) }
        var busy by remember { mutableStateOf(false) }

        // One BillingClient per gate visit; listener handles the purchase result.
        val client = remember {
            BillingClient.newBuilder(context)
                .setListener { result, purchases ->
                    when (result.responseCode) {
                        BillingClient.BillingResponseCode.OK -> {
                            val p = purchases?.firstOrNull {
                                it.products.contains(PRODUCT_ID) &&
                                    it.purchaseState == Purchase.PurchaseState.PURCHASED
                            }
                            if (p != null) {
                                setUnlocked(context, true)
                                onUnlocked()
                            }
                        }
                        BillingClient.BillingResponseCode.USER_CANCELED -> busy = false
                        else -> { busy = false; error = "Purchase failed (${result.debugMessage})" }
                    }
                }
                .enablePendingPurchases(
                    PendingPurchasesParams.newBuilder().enableOneTimeProducts().build(),
                )
                .build()
        }

        LaunchedEffect(Unit) {
            client.startConnection(object : BillingClientStateListener {
                override fun onBillingSetupFinished(result: BillingResult) {
                    if (result.responseCode != BillingClient.BillingResponseCode.OK) {
                        error = "Google Play unavailable (${result.debugMessage})"
                        return
                    }
                    // Restore path first: an existing purchase skips the paywall
                    // without any tap (reinstall / second device).
                    client.queryPurchasesAsync(
                        QueryPurchasesParams.newBuilder()
                            .setProductType(BillingClient.ProductType.INAPP).build(),
                    ) { qr, purchases ->
                        val owned = qr.responseCode == BillingClient.BillingResponseCode.OK &&
                            purchases.any {
                                it.products.contains(PRODUCT_ID) &&
                                    it.purchaseState == Purchase.PurchaseState.PURCHASED
                            }
                        if (owned) {
                            purchases.filter { !it.isAcknowledged }.forEach { acknowledge(client, it) }
                            setUnlocked(context, true)
                            onUnlocked()
                        }
                    }
                    val params = QueryProductDetailsParams.newBuilder()
                        .setProductList(
                            listOf(
                                QueryProductDetailsParams.Product.newBuilder()
                                    .setProductId(PRODUCT_ID)
                                    .setProductType(BillingClient.ProductType.INAPP)
                                    .build(),
                            ),
                        ).build()
                    client.queryProductDetailsAsync(params) { pr, list ->
                        if (pr.responseCode == BillingClient.BillingResponseCode.OK) {
                            details = list.firstOrNull()
                            price = details?.oneTimePurchaseOfferDetails?.formattedPrice
                        } else {
                            error = "Could not load price (${pr.debugMessage})"
                        }
                    }
                }

                override fun onBillingServiceDisconnected() { /* retried on next gate visit */ }
            })
        }
        DisposableEffect(Unit) { onDispose { client.endConnection() } }

        Box(
            modifier = Modifier.fillMaxSize().background(DarkBg),
            contentAlignment = Alignment.Center,
        ) {
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                modifier = Modifier.padding(32.dp),
            ) {
                Text(text = "🎙", fontSize = 64.sp)
                Spacer(modifier = Modifier.height(24.dp))
                Text(
                    text = "Unlock Sassy-Talk",
                    fontSize = 24.sp,
                    fontWeight = FontWeight.Bold,
                    color = Teal,
                )
                Spacer(modifier = Modifier.height(12.dp))
                Text(
                    text = "One-time purchase. Encrypted push-to-talk\nover WiFi, Bluetooth, and relay — forever.",
                    fontSize = 14.sp,
                    color = TextGray,
                    textAlign = TextAlign.Center,
                )
                Spacer(modifier = Modifier.height(32.dp))
                if (busy) {
                    CircularProgressIndicator(color = Teal)
                } else {
                    Button(
                        onClick = {
                            val activity = context as? Activity ?: return@Button
                            val pd = details ?: return@Button
                            busy = true
                            error = null
                            val flowParams = BillingFlowParams.newBuilder()
                                .setProductDetailsParamsList(
                                    listOf(
                                        BillingFlowParams.ProductDetailsParams.newBuilder()
                                            .setProductDetails(pd).build(),
                                    ),
                                ).build()
                            client.launchBillingFlow(activity, flowParams)
                        },
                        enabled = details != null,
                        colors = ButtonDefaults.buttonColors(containerColor = PrimaryBlue),
                        shape = RoundedCornerShape(25.dp),
                        modifier = Modifier.height(52.dp).width(240.dp),
                    ) {
                        Text(
                            text = price?.let { "Unlock — $it" } ?: "Loading…",
                            fontSize = 16.sp,
                        )
                    }
                    Spacer(modifier = Modifier.height(8.dp))
                    TextButton(
                        onClick = {
                            error = null
                            refresh(context) { unlocked ->
                                if (unlocked) onUnlocked() else error = "No purchase found for this Google account"
                            }
                        },
                    ) {
                        Text("Already purchased? Restore", color = TextGray, fontSize = 13.sp)
                    }
                }
                error?.let {
                    Spacer(modifier = Modifier.height(16.dp))
                    Text(
                        text = it,
                        fontSize = 13.sp,
                        color = StatusDisconnected,
                        textAlign = TextAlign.Center,
                    )
                }
            }
        }
    }

    private fun setUnlocked(context: Context, value: Boolean) {
        LicenseStore.prefs(context)?.edit()?.putBoolean(LicenseStore.KEY_UNLOCKED, value)?.apply()
    }

    private fun acknowledge(client: BillingClient, purchase: Purchase) {
        client.acknowledgePurchase(
            AcknowledgePurchaseParams.newBuilder()
                .setPurchaseToken(purchase.purchaseToken).build(),
        ) { r ->
            if (r.responseCode != BillingClient.BillingResponseCode.OK) {
                Log.w(TAG, "acknowledge failed: ${r.debugMessage}")
            }
        }
    }
}
