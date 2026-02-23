package com.sassyconsulting.sassytalkie

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.net.wifi.WifiManager
import android.os.Binder
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.util.Log
import androidx.core.app.NotificationCompat

/**
 * Foreground service that keeps SassyTalkie alive while in use.
 *
 * Responsibilities:
 * 1. Holds a WiFi MulticastLock so the OS delivers multicast UDP packets
 *    (Android filters multicast by default to save battery).
 * 2. Holds a partial WakeLock so audio threads aren't killed when the
 *    screen turns off.
 * 3. Shows a persistent notification so the user knows the radio is on
 *    and the system won't kill us.
 *
 * Lifecycle:
 *   MainActivity.onCreate → startForegroundService(intent)
 *   MainActivity.onDestroy → stopService(intent)
 *   DevicePickerScreen "Join WiFi" → service.acquireMulticastLock()
 *   MainScreen "Disconnect" → service.releaseMulticastLock()
 */
class WalkieService : Service() {

    companion object {
        private const val TAG = "WalkieService"
        private const val CHANNEL_ID = "sassytalkie_radio"
        private const val NOTIFICATION_ID = 1
    }

    inner class LocalBinder : Binder() {
        fun getService(): WalkieService = this@WalkieService
    }

    private val binder = LocalBinder()

    private var multicastLock: WifiManager.MulticastLock? = null
    private var wakeLock: PowerManager.WakeLock? = null

    // ── Service lifecycle ──

    override fun onCreate() {
        super.onCreate()
        Log.i(TAG, "Service created")
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        Log.i(TAG, "Service started")
        startForeground(NOTIFICATION_ID, buildNotification("Radio standby"))
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder = binder

    override fun onDestroy() {
        Log.i(TAG, "Service destroyed")
        releaseMulticastLock()
        releaseWakeLock()
        super.onDestroy()
    }

    // ── Multicast lock ──

    /**
     * Acquire the WiFi MulticastLock. Must be called BEFORE joining multicast.
     * Without this, the WiFi driver silently drops all multicast/broadcast UDP
     * packets on most Android devices.
     */
    fun acquireMulticastLock() {
        if (multicastLock?.isHeld == true) return

        val wifiManager = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        multicastLock = wifiManager.createMulticastLock("SassyTalkie-Multicast").apply {
            setReferenceCounted(false)
            acquire()
        }
        Log.i(TAG, "MulticastLock acquired")

        // Also acquire a partial wake lock so audio threads survive screen-off
        acquireWakeLock()

        // Update notification
        updateNotification("Radio active")
    }

    /**
     * Release the MulticastLock. Call when disconnecting or when the user
     * leaves the walkie-talkie screen.
     */
    fun releaseMulticastLock() {
        multicastLock?.let {
            if (it.isHeld) {
                it.release()
                Log.i(TAG, "MulticastLock released")
            }
        }
        multicastLock = null
        releaseWakeLock()
        updateNotification("Radio standby")
    }

    fun isMulticastLockHeld(): Boolean = multicastLock?.isHeld == true

    // ── Wake lock ──

    private fun acquireWakeLock() {
        if (wakeLock?.isHeld == true) return

        val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
        wakeLock = pm.newWakeLock(
            PowerManager.PARTIAL_WAKE_LOCK,
            "SassyTalkie::RadioWake"
        ).apply {
            // 4-hour max to prevent accidental battery drain if user forgets
            acquire(4 * 60 * 60 * 1000L)
        }
        Log.i(TAG, "WakeLock acquired (4h timeout)")
    }

    private fun releaseWakeLock() {
        wakeLock?.let {
            if (it.isHeld) {
                it.release()
                Log.i(TAG, "WakeLock released")
            }
        }
        wakeLock = null
    }

    // ── Notification ──

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "Sassy-Talk Radio",
                NotificationManager.IMPORTANCE_LOW  // No sound, just persistent icon
            ).apply {
                description = "Keeps the walkie-talkie radio active"
                setShowBadge(false)
            }
            val nm = getSystemService(NotificationManager::class.java)
            nm.createNotificationChannel(channel)
        }
    }

    private fun buildNotification(status: String): Notification {
        val launchIntent = Intent(this, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP
        }
        val pendingIntent = PendingIntent.getActivity(
            this, 0, launchIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Sassy-Talk")
            .setContentText(status)
            .setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setContentIntent(pendingIntent)
            .setOngoing(true)
            .setSilent(true)
            .build()
    }

    fun updateNotification(status: String) {
        try {
            val nm = getSystemService(NotificationManager::class.java)
            nm.notify(NOTIFICATION_ID, buildNotification(status))
        } catch (e: Exception) {
            Log.w(TAG, "Failed to update notification: ${e.message}")
        }
    }
}
