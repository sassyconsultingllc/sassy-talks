package com.sassyconsulting.sassytalkie.license

import android.app.Activity
import android.content.Context
import android.util.Log
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import kotlin.coroutines.resume
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
 * Play-flavor entitlement gate: Google Play Billing purchase, with promo-code
 * redemption for friends & family (relay `/license/promo`).
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

    fun isUnlockedCached(context: Context): Boolean {
        val p = LicenseStore.prefs(context) ?: return false
        if (p.getBoolean(LicenseStore.KEY_UNLOCKED, false)) return true
        return LicensePromo.hasValidReceipt(context)
    }

    /**
     * Reconcile entitlement: promo receipts refresh against the relay worker;
     * otherwise reconnect to Play and reconcile the cached purchase.
     */
    fun refresh(context: Context, onResult: (Boolean) -> Unit = {}) {
        val appContext = context.applicationContext
        if (LicenseStore.prefs(appContext)?.getString(LicenseStore.KEY_KIND, null) == "promo") {
            if (!LicensePromo.hasValidReceipt(appContext)) {
                onResult(false)
                return
            }
            LicensePromo.refreshIfNeeded(appContext) { ok ->
                onResult(ok || LicensePromo.hasValidReceipt(appContext))
            }
            return
        }
        val client = BillingClient.newBuilder(appContext)
            .setListener { _, _ -> } // refresh never launches a flow
            .enablePendingPurchases(
                PendingPurchasesParams.newBuilder().enableOneTimeProducts().build(),
            )
            .build()
        client.startConnection(object : BillingClientStateListener {
            override fun onBillingSetupFinished(result: BillingResult) {
                if (result.responseCode != BillingClient.BillingResponseCode.OK) {
                    onResult(isUnlockedCached(appContext))
                    client.endConnection()
                    return
                }
                client.queryPurchasesAsync(
                    QueryPurchasesParams.newBuilder()
                        .setProductType(BillingClient.ProductType.INAPP).build(),
                ) { qr, purchases ->
                    if (qr.responseCode != BillingClient.BillingResponseCode.OK) {
                        onResult(isUnlockedCached(appContext))
                    } else {
                        val owned = purchases.any {
                            it.products.contains(PRODUCT_ID) &&
                                it.purchaseState == Purchase.PurchaseState.PURCHASED
                        }
                        if (owned) {
                            purchases.filter { it.products.contains(PRODUCT_ID) && !it.isAcknowledged }
                                .forEach { acknowledge(client, it) }
                            setUnlocked(appContext, true)
                        } else if (isUnlockedCached(appContext)) {
                            Log.w(TAG, "Play reports no purchase — revoking cached unlock")
                            setUnlocked(appContext, false)
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
        val appContext = context.applicationContext
        val scope = rememberCoroutineScope()
        var price by remember { mutableStateOf<String?>(null) }
        var details by remember { mutableStateOf<ProductDetails?>(null) }
        var error by remember { mutableStateOf<String?>(null) }
        var busy by remember { mutableStateOf(false) }
        var promoInput by remember { mutableStateOf("") }
        var promoBusy by remember { mutableStateOf(false) }
        var catalogLoading by remember { mutableStateOf(true) }
        var catalogAttempt by remember { mutableIntStateOf(0) }

        // One BillingClient per gate visit; listener handles the purchase result.
        val clientHolder = remember { arrayOfNulls<BillingClient>(1) }
        val client = remember {
            BillingClient.newBuilder(appContext)
                .setListener { result, purchases ->
                    val billingClient = clientHolder[0] ?: return@setListener
                    scope.launch(Dispatchers.Main.immediate) {
                        when (result.responseCode) {
                            BillingClient.BillingResponseCode.OK -> {
                                val p = purchases?.firstOrNull {
                                    it.products.contains(PRODUCT_ID) &&
                                        it.purchaseState == Purchase.PurchaseState.PURCHASED
                                }
                                if (p != null) {
                                    acknowledge(billingClient, p)
                                    setUnlocked(appContext, true)
                                    busy = false
                                    onUnlocked()
                                } else {
                                    busy = false
                                }
                            }
                            BillingClient.BillingResponseCode.USER_CANCELED -> busy = false
                            BillingClient.BillingResponseCode.ITEM_ALREADY_OWNED -> {
                                busy = false
                                refresh(appContext) { unlocked ->
                                    scope.launch(Dispatchers.Main.immediate) {
                                        if (unlocked) onUnlocked()
                                        else error = "Purchase exists but could not be restored"
                                    }
                                }
                            }
                            else -> {
                                busy = false
                                error = "Purchase failed (${result.debugMessage})"
                            }
                        }
                    }
                }
                .enablePendingPurchases(
                    PendingPurchasesParams.newBuilder().enableOneTimeProducts().build(),
                )
                .build()
                .also { clientHolder[0] = it }
        }

        fun applyCatalogResult(
            product: ProductDetails?,
            err: String?,
        ) {
            catalogLoading = false
            details = product
            price = product?.oneTimePurchaseOfferDetails?.formattedPrice
            if (product == null && err != null) error = err
        }

        LaunchedEffect(catalogAttempt) {
            if (LicensePromo.hasValidReceipt(appContext)) {
                setUnlocked(appContext, true)
                onUnlocked()
                return@LaunchedEffect
            }
            catalogLoading = true
            error = null
            details = null
            price = null
            client.endConnection()

            val timedOut = withTimeoutOrNull(15_000L) {
                suspendCancellableCoroutine { cont ->
                    client.startConnection(object : BillingClientStateListener {
                        override fun onBillingSetupFinished(result: BillingResult) {
                            if (result.responseCode != BillingClient.BillingResponseCode.OK) {
                                scope.launch(Dispatchers.Main.immediate) {
                                    applyCatalogResult(
                                        null,
                                        "Google Play unavailable (${result.debugMessage})",
                                    )
                                }
                                if (cont.isActive) cont.resume(Unit)
                                return
                            }
                            // Restore path: existing purchase skips the paywall.
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
                                    purchases.filter { !it.isAcknowledged }
                                        .forEach { acknowledge(client, it) }
                                    scope.launch(Dispatchers.Main.immediate) {
                                        catalogLoading = false
                                        setUnlocked(appContext, true)
                                        onUnlocked()
                                    }
                                    if (cont.isActive) cont.resume(Unit)
                                    return@queryPurchasesAsync
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
                                    scope.launch(Dispatchers.Main.immediate) {
                                        when {
                                            pr.responseCode != BillingClient.BillingResponseCode.OK ->
                                                applyCatalogResult(
                                                    null,
                                                    "Could not load price (${pr.debugMessage})",
                                                )
                                            list.isEmpty() ->
                                                applyCatalogResult(
                                                    null,
                                                    "Unlock product not found in Play Store. " +
                                                        "Confirm \"$PRODUCT_ID\" is published, or tap Restore.",
                                                )
                                            else -> applyCatalogResult(list.first(), null)
                                        }
                                    }
                                    if (cont.isActive) cont.resume(Unit)
                                }
                            }
                        }

                        override fun onBillingServiceDisconnected() {
                            if (cont.isActive) {
                                scope.launch(Dispatchers.Main.immediate) {
                                    applyCatalogResult(
                                        null,
                                        "Lost connection to Google Play — tap Retry",
                                    )
                                }
                                cont.resume(Unit)
                            }
                        }
                    })
                }
            }
            if (timedOut == null && catalogLoading) {
                applyCatalogResult(
                    null,
                    "Google Play is taking too long. Check your connection and tap Retry.",
                )
            }
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
                when {
                    busy || catalogLoading -> {
                        CircularProgressIndicator(color = Teal)
                        Spacer(modifier = Modifier.height(12.dp))
                        Text(
                            text = if (busy) "Processing purchase…" else "Connecting to Google Play…",
                            fontSize = 13.sp,
                            color = TextGray,
                        )
                    }
                    details != null -> {
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
                                val launch = client.launchBillingFlow(activity, flowParams)
                                if (launch.responseCode != BillingClient.BillingResponseCode.OK) {
                                    busy = false
                                    error = "Could not start purchase (${launch.debugMessage})"
                                }
                            },
                            colors = ButtonDefaults.buttonColors(containerColor = PrimaryBlue),
                            shape = RoundedCornerShape(25.dp),
                            modifier = Modifier.height(52.dp).width(240.dp),
                        ) {
                            Text(
                                text = price?.let { "Unlock — $it" } ?: "Unlock",
                                fontSize = 16.sp,
                            )
                        }
                    }
                    else -> {
                        Button(
                            onClick = { catalogAttempt++ },
                            colors = ButtonDefaults.buttonColors(containerColor = PrimaryBlue),
                            shape = RoundedCornerShape(25.dp),
                            modifier = Modifier.height(52.dp).width(240.dp),
                        ) {
                            Text("Retry", fontSize = 16.sp)
                        }
                    }
                }
                if (!catalogLoading && !busy) {
                    Spacer(modifier = Modifier.height(8.dp))
                    TextButton(
                        onClick = {
                            error = null
                            busy = true
                            refresh(appContext) { unlocked ->
                                scope.launch(Dispatchers.Main.immediate) {
                                    busy = false
                                    if (unlocked) onUnlocked()
                                    else error = "No purchase found for this Google account"
                                }
                            }
                        },
                    ) {
                        Text("Already purchased? Restore", color = TextGray, fontSize = 13.sp)
                    }
                }
                if (!busy) {
                    Spacer(modifier = Modifier.height(24.dp))
                    Text(
                        text = "Have a promo code?",
                        fontSize = 13.sp,
                        color = TextGray,
                    )
                    Spacer(modifier = Modifier.height(8.dp))
                    OutlinedTextField(
                        value = promoInput,
                        onValueChange = { promoInput = it.uppercase() },
                        placeholder = { Text("Promo code", color = TextGray) },
                        singleLine = true,
                        enabled = !promoBusy,
                        keyboardOptions = KeyboardOptions(
                            capitalization = KeyboardCapitalization.Characters,
                        ),
                        colors = OutlinedTextFieldDefaults.colors(
                            focusedTextColor = Teal,
                            unfocusedTextColor = Teal,
                            focusedBorderColor = PrimaryBlue,
                            unfocusedBorderColor = TextGray,
                        ),
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Spacer(modifier = Modifier.height(12.dp))
                    if (promoBusy) {
                        CircularProgressIndicator(color = Teal)
                    } else {
                        Button(
                            onClick = {
                                promoBusy = true
                                error = null
                                val code = promoInput.trim()
                                scope.launch {
                                    val res = withContext(Dispatchers.IO) {
                                        LicensePromo.redeemBlocking(appContext, code)
                                    }
                                    promoBusy = false
                                    when (res) {
                                        LicensePromo.RedeemResult.Ok -> {
                                            setUnlocked(appContext, true)
                                            onUnlocked()
                                        }
                                        LicensePromo.RedeemResult.InvalidFormat ->
                                            error = "Enter a valid promo code (6–40 characters)"
                                        LicensePromo.RedeemResult.NetworkError ->
                                            error = "Can't reach the license server — check your connection"
                                        is LicensePromo.RedeemResult.Rejected ->
                                            error = res.message
                                    }
                                }
                            },
                            enabled = promoInput.isNotBlank(),
                            colors = ButtonDefaults.buttonColors(containerColor = PrimaryBlue),
                            shape = RoundedCornerShape(25.dp),
                            modifier = Modifier.height(48.dp).width(240.dp),
                        ) {
                            Text("Redeem promo", fontSize = 15.sp)
                        }
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
