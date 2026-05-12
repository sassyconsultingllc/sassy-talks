package com.sassyconsulting.sassytalkie

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Binder
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import com.sassyconsulting.sassytalkie.service.BluetoothTransport
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

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

    private val serviceScope = CoroutineScope(Dispatchers.Default)
    private var cohortSnapshotJob: Job? = null

    // BLE + RFCOMM
    var bleSignaling: BleSignalingService? = null
        private set
    var btTransport: BluetoothTransport? = null
        private set
    var pttCoordinator: PttCoordinator? = null
        private set
    private var bleInitialized = false

    // ── Service lifecycle ──

    override fun onCreate() {
        super.onCreate()
        Log.i(TAG, "Service created")
        createNotificationChannel()
        // Snapshotter is keyed to service lifetime, not multicast. The inner
        // getActiveCohortId() guard makes it a no-op when no channel has an
        // active session — so it's safe to run regardless of transport.
        startCohortSnapshotter()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        Log.i(TAG, "Service started")
        try {
            // On API 34+ the foreground service type must be passed explicitly or
            // the system raises MissingForegroundServiceTypeException and kills us.
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                ServiceCompat.startForeground(
                    this,
                    NOTIFICATION_ID,
                    buildNotification("Radio standby"),
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK or
                        ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
                )
            } else {
                startForeground(NOTIFICATION_ID, buildNotification("Radio standby"))
            }
        } catch (e: Exception) {
            // Foreground promotion failed (denied permission, policy, etc). Stop
            // the service cleanly rather than let the OS crash the whole app.
            Log.e(TAG, "startForeground failed: ${e.message}", e)
            stopSelf()
            return START_NOT_STICKY
        }
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder = binder

    override fun onDestroy() {
        Log.i(TAG, "Service destroyed")
        stopCohortSnapshotter()
        serviceScope.cancel()
        shutdownBleTransport()
        releaseMulticastLock()
        releaseWakeLock()
        // Explicitly remove the ongoing notification so it doesn't linger in the
        // shade after the service itself is gone.
        try {
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        } catch (e: Exception) {
            Log.w(TAG, "stopForeground failed: ${e.message}")
        }
        super.onDestroy()
    }

    // ── BLE + RFCOMM init ──

    /**
     * Initialize BLE signaling + RFCOMM transport.
     * Call after SassyTalkNative.init() succeeds and BT permissions are granted.
     */
    @android.annotation.SuppressLint("MissingPermission")
    fun initBleTransport() {
        if (bleInitialized) {
            Log.i(TAG, "BLE transport already initialized; skipping")
            return
        }

        val adapter = (getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager)?.adapter
        if (adapter == null || !adapter.isEnabled) {
            Log.w(TAG, "Bluetooth not available or not enabled")
            return
        }

        val ble = BleSignalingService(this, adapter)
        val bt = BluetoothTransport(this)
        val coord = PttCoordinator(ble, bt)

        // Start BLE
        ble.startServer()
        ble.startAdvertising()
        ble.startScanning()

        // Start RFCOMM listener
        bt.startAcceptThread()

        bleSignaling = ble
        btTransport = bt
        pttCoordinator = coord

        // Wire BT transport reference so SassyTalkNative.pttStart() can start TX pump
        SassyTalkNative.bluetoothTransport = bt

        Log.i(TAG, "BLE + RFCOMM transport initialized")
        bleInitialized = true
    }

    private fun shutdownBleTransport() {
        pttCoordinator?.shutdown()
        btTransport?.shutdown()
        bleSignaling?.shutdown()

        SassyTalkNative.bluetoothTransport = null
        pttCoordinator = null
        btTransport = null
        bleSignaling = null
        bleInitialized = false

        Log.i(TAG, "BLE + RFCOMM transport shut down")
    }

    // ── Cohort participant snapshotter ──

    private fun startCohortSnapshotter() {
        if (cohortSnapshotJob?.isActive == true) return
        cohortSnapshotJob = serviceScope.launch {
            while (isActive) {
                try {
                    val users = SassyTalkNative.getUsers()
                    if (users.isNotEmpty()) {
                        val arr = org.json.JSONArray()
                        for (u in users) {
                            val o = org.json.JSONObject()
                            o.put("id", u.id)
                            o.put("name", u.name)
                            arr.put(o)
                        }
                        val payload = arr.toString()
                        for (ch in 1..8) {
                            val cid = SassyTalkNative.getActiveCohortId(ch)
                            if (cid.isNotEmpty()) {
                                SassyTalkNative.snapshotCohortParticipants(ch, payload)
                            }
                        }
                    }
                } catch (e: Exception) {
                    Log.w(TAG, "cohort snapshot failed: ${e.message}")
                }
                delay(30_000)
            }
        }
    }

    private fun stopCohortSnapshotter() {
        cohortSnapshotJob?.cancel()
        cohortSnapshotJob = null
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
                NotificationManager.IMPORTANCE_DEFAULT
            ).apply {
                description = "Keeps the walkie-talkie radio active"
                setShowBadge(true)
                lockscreenVisibility = Notification.VISIBILITY_PUBLIC
                // No sound for ongoing updates — only visual presence
                setSound(null, null)
                enableVibration(false)
            }
            val nm = getSystemService(NotificationManager::class.java)
            nm.createNotificationChannel(channel)
        }
    }

    private fun buildNotification(status: String, showPttAction: Boolean = false): Notification {
        val launchIntent = Intent(this, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
        }
        val pendingIntent = PendingIntent.getActivity(
            this, 0, launchIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        val builder = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Sassy-Talk")
            .setContentText(status)
            .setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setContentIntent(pendingIntent)
            .setOngoing(true)
            .setSilent(true)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .setShowWhen(true)
            .setWhen(System.currentTimeMillis())
            .setNumber(1)
            .setBadgeIconType(NotificationCompat.BADGE_ICON_SMALL)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)

        // Add PTT action if lock screen PTT is enabled
        if (showPttAction) {
            val pttIntent = Intent(this, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_SINGLE_TOP
                putExtra("ptt_from_notification", true)
            }
            val pttPendingIntent = PendingIntent.getActivity(
                this, 1, pttIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
            )
            builder.addAction(
                android.R.drawable.ic_btn_speak_now,
                "Open PTT",
                pttPendingIntent
            )
        }

        return builder.build()
    }

    fun updateNotification(status: String) {
        val lockScreenPtt = try {
            getSharedPreferences("sassy_settings", Context.MODE_PRIVATE)
                .getBoolean("lock_screen_ptt", false)
        } catch (_: Exception) { false }

        try {
            val nm = getSystemService(NotificationManager::class.java)
            nm.notify(NOTIFICATION_ID, buildNotification(status, showPttAction = lockScreenPtt))
        } catch (e: Exception) {
            Log.w(TAG, "Failed to update notification: ${e.message}")
        }
    }
}
