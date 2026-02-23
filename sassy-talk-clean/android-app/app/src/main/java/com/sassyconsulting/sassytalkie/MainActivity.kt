package com.sassyconsulting.sassytalkie

import android.Manifest
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.IBinder
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.core.content.ContextCompat
import com.sassyconsulting.sassytalkie.ui.theme.SassyTalkTheme
import com.sassyconsulting.sassytalkie.ui.AppNavigation

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

    private val requiredPermissions: Array<String>
        get() {
            val perms = mutableListOf(
                Manifest.permission.RECORD_AUDIO,
                Manifest.permission.CAMERA,
            )
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
        super.onCreate(savedInstanceState)

        setContent {
            SassyTalkTheme {
                Surface(
                    modifier = Modifier.fillMaxSize(),
                    color = MaterialTheme.colorScheme.background
                ) {
                    AppNavigation(
                        permissionsGranted = permissionsGranted.value,
                        walkieService = walkieService.value,
                        onRequestPermissions = { requestAllPermissions() }
                    )
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
    }

    override fun onStop() {
        super.onStop()
        SassyTalkNative.pttStop()
        // Don't stop the service here — it should keep running in the background
        // so audio keeps working when the screen is off.
        // Only unbind so we don't leak the connection.
        try {
            unbindService(serviceConnection)
        } catch (_: Exception) { }
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
