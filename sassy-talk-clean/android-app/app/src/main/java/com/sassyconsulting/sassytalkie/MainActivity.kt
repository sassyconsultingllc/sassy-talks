package com.sassyconsulting.sassytalkie

import android.Manifest
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.os.Build
import android.os.Bundle
import android.os.IBinder
import android.net.Uri
import android.util.Log
import android.util.Rational
import android.view.KeyEvent
import android.view.WindowManager
import android.app.PictureInPictureParams
import androidx.activity.ComponentActivity
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import com.sassyconsulting.sassytalkie.debug.DebugOverlay
import com.sassyconsulting.sassytalkie.debug.DiagnosticsPrefs
import com.sassyconsulting.sassytalkie.input.HardwarePttController
import com.sassyconsulting.sassytalkie.ui.theme.SassyTalkTheme
import com.sassyconsulting.sassytalkie.ui.AppNavigation
import com.sassyconsulting.sassytalkie.ui.PipRadioOverlay
import com.sassyconsulting.sassytalkie.ui.theme.BgDark

/**
 * Main activity — handles permission sequencing and foreground service lifecycle.
 *
 * Startup sequence:
 *   1. Request all required permissions
 *   2. Once ALL granted, set permissionsGranted = true
 *   3. AppNavigation observes permissionsGranted before calling nativeInit()
 *   4. Start foreground service for multicast lock + wake lock
 *
 * This eliminates the race condition where nativeInit() tried to create
 * AudioRecord before RECORD_AUDIO was granted.
 */
class MainActivity : ComponentActivity() {

    companion object {
        private const val TAG = "MainActivity"
    }

    // Observable state that AppNavigation reads
    val permissionsGranted = mutableStateOf(false)
    val walkieService = mutableStateOf<WalkieService?>(null)
    val pendingShareUri = mutableStateOf<android.net.Uri?>(null)
    val inPictureInPictureMode = mutableStateOf(false)
    /** True while the main radio screen is active — enables auto-PiP on home press. */
    val pipEligible = mutableStateOf(false)

    // ── Hardware push-to-talk ──
    // Routes physical PTT buttons + Bluetooth PTT accessories through the
    // PttCoordinator (BLE wake, RFCOMM pump, reach watchdog). Created in
    // onCreate, armed in onStart, disarmed in onStop.
    private val hardwarePtt: HardwarePttController by lazy { HardwarePttController(this) }

    private val requiredPermissions: Array<String>
        get() {
            val perms = mutableListOf(
                Manifest.permission.RECORD_AUDIO,
                Manifest.permission.CAMERA,
            )
            // Android 12+ requires runtime Bluetooth permissions
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                perms.add(Manifest.permission.BLUETOOTH_CONNECT)
                perms.add(Manifest.permission.BLUETOOTH_SCAN)
                perms.add(Manifest.permission.BLUETOOTH_ADVERTISE)
            } else {
                // Android 7–11 (API 24–30): BLE scanning legally requires
                // location permission or BluetoothLeScanner.startScan returns
                // no results. (On 31+ neverForLocation removes this need.)
                perms.add(Manifest.permission.ACCESS_FINE_LOCATION)
            }
            // Android 13+ requires POST_NOTIFICATIONS for foreground service notification
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                perms.add(Manifest.permission.POST_NOTIFICATIONS)
            }
            return perms.toTypedArray()
        }

    private val requestPermissionsLauncher = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { results ->
        Log.i(TAG, "Permission results: $results")
        checkAllPermissions()
    }

    // ── Service binding ──

    private val serviceConnection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, service: IBinder?) {
            val binder = service as WalkieService.LocalBinder
            walkieService.value = binder.getService()
            Log.i(TAG, "WalkieService bound")
        }

        override fun onServiceDisconnected(name: ComponentName?) {
            walkieService.value = null
            Log.w(TAG, "WalkieService unbound")
        }
    }

    // ── Activity lifecycle ──

    override fun onCreate(savedInstanceState: Bundle?) {
        installSplashScreen()
        super.onCreate(savedInstanceState)

        // Edge-to-edge with the non-deprecated API (window.statusBarColor /
        // navigationBarColor are deprecated as of API 35). The app is
        // always-dark, so force dark bars; the transparent scrim lets the
        // Compose background draw full-bleed behind them. Content is kept
        // clear of the bars by safeDrawing padding on the root Box below.
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(android.graphics.Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(android.graphics.Color.TRANSPARENT),
        )

        // Diagnostics overlay toggle — persisted, honoured in release builds.
        DiagnosticsPrefs.init(this)

        // Block screenshots on the live radio screen; Auth/QR screens re-enable
        // capture via setScreenshotsAllowed() so users can photograph QR codes.
        if (BuildConfig.NO_SCREENSHOTS) {
            setScreenshotsAllowed(false)
        }

        // Capture the initial deep-link URI if we were launched via VIEW intent.
        captureShareIntent(intent)

        setContent {
            SassyTalkTheme {
                val pipActive by inPictureInPictureMode
                val hwTransmitting by hardwarePtt.transmitting.collectAsState()

                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .background(BgDark)
                        .windowInsetsPadding(WindowInsets.safeDrawing),
                ) {
                    if (pipActive) {
                        PipRadioOverlay(isTransmitting = hwTransmitting)
                    } else {
                        AppNavigation(
                            permissionsGranted = permissionsGranted.value,
                            walkieService = walkieService.value,
                            onRequestPermissions = { requestAllPermissions() },
                            pendingShareUri = pendingShareUri.value,
                            onShareConsumed = { pendingShareUri.value = null },
                            onPipEligibilityChanged = { eligible ->
                                pipEligible.value = eligible
                                updatePipAutoEnter()
                            },
                        )
                    }
                    val diagOn by DiagnosticsPrefs.overlayEnabled.collectAsState()
                    if (!pipActive && (BuildConfig.DEBUG || diagOn)) {
                        DebugOverlay(
                            modifier = Modifier
                                .align(Alignment.TopEnd)
                                .padding(8.dp)
                                .width(280.dp),
                        )
                    }
                }
            }
        }

        // Check permissions — if already granted from a prior run, we skip the dialog
        checkAllPermissions()
    }

    override fun onStart() {
        super.onStart()
        // Start + bind the foreground service
        val intent = Intent(this, WalkieService::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            startForegroundService(intent)
        } else {
            startService(intent)
        }
        bindService(intent, serviceConnection, Context.BIND_AUTO_CREATE)

        hardwarePtt.pttCoordinatorProvider = { walkieService.value?.pttCoordinator }

        // Arm hardware/BT PTT while the Activity is in the foreground. The
        // controller registers its MediaSession here so BT PTT pucks keep
        // working when we're subsequently backgrounded behind WalkieService.
        try { hardwarePtt.enable() } catch (e: Exception) {
            Log.w(TAG, "hardwarePtt enable failed: ${e.message}")
        }
    }

    override fun onStop() {
        super.onStop()
        // Disarm hardware PTT (releases the MediaSession, stops any held TX).
        try { hardwarePtt.disable() } catch (e: Exception) {
            Log.w(TAG, "hardwarePtt disable failed: ${e.message}")
        }
        walkieService.value?.pttCoordinator?.onPttReleased() ?: SassyTalkNative.pttStop()
        // Don't stop the service here — it should keep running in the background
        // so audio keeps working when the screen is off.
        // Only unbind so we don't leak the connection.
        try {
            unbindService(serviceConnection)
        } catch (_: Exception) { }
    }

    override fun onResume() {
        super.onResume()
        updatePipAutoEnter()
    }

    override fun onUserLeaveHint() {
        super.onUserLeaveHint()
        if (!pipEligible.value || inPictureInPictureMode.value) return
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        try {
            enterPictureInPictureMode(buildPipParams())
        } catch (e: Exception) {
            Log.w(TAG, "enterPictureInPictureMode failed: ${e.message}")
        }
    }

    private fun buildPipParams(): PictureInPictureParams {
        val builder = PictureInPictureParams.Builder()
            .setAspectRatio(Rational(2, 3))
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            builder.setAutoEnterEnabled(pipEligible.value)
        }
        return builder.build()
    }

    private fun updatePipAutoEnter() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return
        try {
            setPictureInPictureParams(buildPipParams())
        } catch (e: Exception) {
            Log.w(TAG, "setPictureInPictureParams failed: ${e.message}")
        }
    }

    override fun onPictureInPictureModeChanged(
        isInPictureInPictureMode: Boolean,
        newConfig: Configuration,
    ) {
        super.onPictureInPictureModeChanged(isInPictureInPictureMode, newConfig)
        inPictureInPictureMode.value = isInPictureInPictureMode
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        captureShareIntent(intent)
    }

    private fun captureShareIntent(i: Intent?) {
        if (i?.action != Intent.ACTION_VIEW) return
        // Prefer dataString — it preserves the #fragment on some Android builds
        // where Intent.getData() drops it during App Link dispatch.
        val raw = i.dataString ?: i.data?.toString() ?: return
        val uri = Uri.parse(raw)
        val ok = (uri.scheme == "https" &&
            uri.host == "relay.sassyconsultingllc.com" &&
            (uri.path ?: "").startsWith("/v/")) ||
            (uri.scheme == SessionShareLink.APP_SCHEME &&
                uri.host == "v" &&
                (uri.path ?: "").length > 1)
        if (ok) pendingShareUri.value = uri
    }

    /**
     * Release builds block screenshots on the live radio screen (FLAG_SECURE).
     * Auth / QR screens allow capture so remote users can photograph a QR code
     * when invite links fail.
     */
    fun setScreenshotsAllowed(allowed: Boolean) {
        if (!BuildConfig.NO_SCREENSHOTS) return
        if (allowed) {
            window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
        } else {
            window.setFlags(
                WindowManager.LayoutParams.FLAG_SECURE,
                WindowManager.LayoutParams.FLAG_SECURE,
            )
        }
    }

    // ── Hardware key delegation ──
    // Delegate to HardwarePttController first; consume only when it handled the
    // configured PTT key. Returning super.* otherwise preserves all normal
    // system key behavior (volume, back, etc.).

    override fun onKeyDown(keyCode: Int, event: KeyEvent?): Boolean {
        if (hardwarePtt.onKeyDown(keyCode, event)) return true
        return super.onKeyDown(keyCode, event)
    }

    override fun onKeyUp(keyCode: Int, event: KeyEvent?): Boolean {
        if (hardwarePtt.onKeyUp(keyCode, event)) return true
        return super.onKeyUp(keyCode, event)
    }

    override fun onDestroy() {
        super.onDestroy()
        // Full shutdown: stop native + stop service
        SassyTalkNative.shutdown()
        stopService(Intent(this, WalkieService::class.java))
    }

    // ── Permission helpers ──

    private fun checkAllPermissions() {
        val allGranted = requiredPermissions.all { perm ->
            ContextCompat.checkSelfPermission(this, perm) == PackageManager.PERMISSION_GRANTED
        }

        if (allGranted) {
            Log.i(TAG, "All permissions granted")
            permissionsGranted.value = true
        } else {
            Log.i(TAG, "Some permissions missing — requesting")
            requestAllPermissions()
        }
    }

    private fun requestAllPermissions() {
        val missing = requiredPermissions.filter { perm ->
            ContextCompat.checkSelfPermission(this, perm) != PackageManager.PERMISSION_GRANTED
        }
        if (missing.isNotEmpty()) {
            requestPermissionsLauncher.launch(missing.toTypedArray())
        }
    }
}
