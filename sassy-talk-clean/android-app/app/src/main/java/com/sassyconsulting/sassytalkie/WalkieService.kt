package com.sassyconsulting.sassytalkie

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Binder
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import com.sassyconsulting.sassytalkie.debug.AudioTelemetry
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
        const val ACTION_TOGGLE_PTT = "com.sassyconsulting.sassytalkie.action.TOGGLE_PTT"
        /** Sent by [SassyTalkFcmService] when an inbound wake push arrives. */
        const val ACTION_WAKE = "com.sassyconsulting.sassytalkie.action.WAKE"
        const val EXTRA_ROOM = "room"
    }

    inner class LocalBinder : Binder() {
        fun getService(): WalkieService = this@WalkieService
    }

    private val binder = LocalBinder()

    private var multicastLock: WifiManager.MulticastLock? = null
    private var wakeLock: PowerManager.WakeLock? = null

    // SupervisorJob so a failure in one child (cohort snapshotter, telemetry
    // bridge, kickCellularReconnect) doesn't cascade-cancel the others.
    // Without it, a single uncaught exception silently kills every long-
    // lived coroutine in this service.
    private val serviceScope = CoroutineScope(Dispatchers.Default + kotlinx.coroutines.SupervisorJob())

    private var cohortSnapshotJob: Job? = null
    private var telemetryBridgeJob: Job? = null

    // Tracks the toggle state for the notification's PTT action — flipped only
    // when the action button is tapped from the shade, NOT for in-app PTT.
    @Volatile
    private var notificationPttActive = false
    private var pttToggleReceiver: BroadcastReceiver? = null

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
        registerPttToggleReceiver()
        // Snapshotter is keyed to service lifetime, not multicast.
        // getActiveCohortId() guard makes it a no-op when no channel has an
        // active session — so it's safe to run regardless of transport.
        startCohortSnapshotter()
        startTelemetryBridge()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        Log.i(TAG, "Service started action=${intent?.action}")

        // Wake-intent fast path: skip the foreground promotion entirely.
        // startForeground with FOREGROUND_SERVICE_TYPE_MICROPHONE from a
        // background-launched service throws on Android 14+ (BackgroundService
        // StartNotAllowedException). The MainActivity launch fired alongside
        // the wake handles real foreground promotion via the standard
        // onStart path; this branch just runs the reconnect work.
        if (intent?.action == ACTION_WAKE) {
            val room = intent.getStringExtra(EXTRA_ROOM)
            Log.i(TAG, "WAKE intent received room=$room")
            serviceScope.launch { kickCellularReconnect() }
            return START_NOT_STICKY
        }

        try {
            // On API 34+ the foreground service type must be passed explicitly or
            // the system raises MissingForegroundServiceTypeException and kills us.
            //
            // FOREGROUND_SERVICE_TYPE_MICROPHONE is OR'd in for API 34+ so the new
            // PttAudioPipeline (com.sassyconsulting.sassytalkie.audio) can keep
            // AudioRecord alive in the background. The Rust pipeline historically
            // captured under mediaPlayback, which API 34 deprecates for mic input.
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                ServiceCompat.startForeground(
                    this,
                    NOTIFICATION_ID,
                    buildNotification("Radio standby"),
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK or
                        ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE or
                        ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
                )
            } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
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

    /**
     * Force the cellular relay client to reconnect if it's disconnected.
     * Safe to call from any thread — guards against double-connect.
     *
     * On a cold-start triggered BY an FCM wake push (process killed, FCM
     * delivery spawns WalkieService), PttCoordinator + CellularWebSocketClient
     * haven't been wired yet — they're set up by AppNavigation after
     * MainActivity launches. Polling for up to 15s gives the normal startup
     * a chance to bring them up so the wake actually re-attaches the relay.
     */
    private suspend fun kickCellularReconnect() {
        val deadline = System.currentTimeMillis() + 15_000L
        while (System.currentTimeMillis() < deadline) {
            val coord = pttCoordinator
            val client = coord?.cellularClient
            if (client != null) {
                if (client.isConnected()) {
                    Log.d(TAG, "kickCellularReconnect: already connected, skipping")
                    return
                }
                Log.i(TAG, "kickCellularReconnect: triggering connect()")
                try { client.connect() } catch (t: Throwable) {
                    Log.w(TAG, "cellular connect() threw: ${t.message}")
                }
                return
            }
            kotlinx.coroutines.delay(500)
        }
        Log.w(TAG, "kickCellularReconnect: gave up waiting for PttCoordinator after 15s")
    }

    /**
     * Force a full WS teardown + reconnect. Unlike [kickCellularReconnect],
     * this does NOT skip when isConnected() returns true — it explicitly
     * disconnects first. Use this after the Rust session room changes
     * (QR generate / QR scan / share-link import), where the WS may still
     * appear "connected" to the OLD room and silently drop everything sent
     * for the new one.
     *
     * Public so SessionShareLink + QRAuthScreen can invoke it directly via
     * a service binder reference.
     */
    fun forceCellularReconnect() {
        serviceScope.launch {
            // Brief window in case pttCoordinator is mid-initialization — but
            // don't wait long. If the coordinator doesn't exist this is the
            // Auth-screen-first-generate case: there's no live WS to bounce.
            // The natural Auth → Main navigation will spin up a fresh
            // cellular client against whatever room Rust has set, which
            // already includes the new session_id (Rust's set_cellular_room
            // ran synchronously inside nativeGenerateChannelQR before this
            // Kotlin path fires). Bail quickly so we don't sit in a 15 s
            // dead wait every time the host generates from a cold start.
            val deadline = System.currentTimeMillis() + 2_000L
            while (System.currentTimeMillis() < deadline) {
                val client = pttCoordinator?.cellularClient
                if (client != null) {
                    try {
                        Log.i(TAG, "forceCellularReconnect: disconnect → reconnect for room change")
                        client.disconnect()
                    } catch (t: Throwable) {
                        Log.w(TAG, "force disconnect threw: ${t.message}")
                    }
                    // Small grace gap so the prior socket close completes before
                    // we open the new one; otherwise OkHttp can collapse the
                    // second connect into the closing of the first.
                    kotlinx.coroutines.delay(150)
                    try {
                        client.connect()
                    } catch (t: Throwable) {
                        Log.w(TAG, "force connect threw: ${t.message}")
                    }
                    return@launch
                }
                kotlinx.coroutines.delay(200)
            }
            // No coordinator yet — that's fine. Rust already updated the
            // room, and the next time MainScreen mounts autoConnect will
            // create a fresh cellular client against the new room.
            Log.d(TAG, "forceCellularReconnect: no PttCoordinator yet — relying on Main-mount autoConnect to pick up new room")
        }
    }

    override fun onBind(intent: Intent?): IBinder = binder

    override fun onDestroy() {
        Log.i(TAG, "Service destroyed")
        stopCohortSnapshotter()
        stopTelemetryBridge()
        serviceScope.cancel()
        shutdownBleTransport()
        releaseMulticastLock()
        releaseWakeLock()
        unregisterPttToggleReceiver()
        // Explicitly remove the ongoing notification so it doesn't linger in the
        // shade after the service itself is gone.
        try {
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        } catch (e: Exception) {
            Log.w(TAG, "stopForeground failed: ${e.message}")
        }
        super.onDestroy()
    }

    // ── Notification PTT toggle receiver ──

    /**
     * Lets the user start/stop a transmission from the notification shade
     * without opening the app. Required by the FOREGROUND_SERVICE_MICROPHONE
     * demo: the user must be able to acknowledge and act on the foreground
     * service from the notification itself.
     */
    private fun registerPttToggleReceiver() {
        if (pttToggleReceiver != null) return
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                if (intent.action != ACTION_TOGGLE_PTT) return
                handleNotificationPttToggle()
            }
        }
        val filter = IntentFilter(ACTION_TOGGLE_PTT)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            @Suppress("UnspecifiedRegisterReceiverFlag")
            registerReceiver(receiver, filter)
        }
        pttToggleReceiver = receiver
    }

    private fun unregisterPttToggleReceiver() {
        pttToggleReceiver?.let {
            try { unregisterReceiver(it) } catch (_: Exception) {}
        }
        pttToggleReceiver = null
        notificationPttActive = false
    }

    private fun handleNotificationPttToggle() {
        try {
            val coord = pttCoordinator
            if (notificationPttActive) {
                if (coord != null) coord.onPttReleased() else SassyTalkNative.pttStop()
                notificationPttActive = false
                updateNotification("Radio standby")
            } else {
                val started = if (coord != null) {
                    coord.onPttPressed()
                } else {
                    SassyTalkNative.pttStart()
                    true
                }
                if (!started) {
                    updateNotification("Radio standby — no peers")
                    return
                }
                notificationPttActive = true
                updateNotification("Transmitting…")
            }
        } catch (e: Exception) {
            Log.w(TAG, "Notification PTT toggle failed: ${e.message}")
            notificationPttActive = false
            updateNotification("Radio standby")
        }
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

    // ── AudioTelemetry network bridge ──
    //
    // The new debug overlay (com.sassyconsulting.sassytalkie.debug.DebugOverlay)
    // surfaces transport state in its NET section. The Rust pipeline owns the
    // canonical transport, so we poll SassyTalkNative once per second and push
    // into the telemetry singleton. Cheap (a few JNI string calls); only runs
    // for service lifetime. If/when PttAudioPipeline takes over capture, this
    // keeps working unchanged.

    private fun startTelemetryBridge() {
        if (telemetryBridgeJob?.isActive == true) return
        telemetryBridgeJob = serviceScope.launch {
            while (isActive) {
                try {
                    val path = SassyTalkNative.getTransportName().ifBlank { "offline" }
                    val wsState = if (SassyTalkNative.isConnected()) "connected" else "idle"
                    AudioTelemetry.updateNetwork(
                        path = path,
                        wsState = wsState,
                        rttMs = null,
                        hbAgoMs = null,
                    )
                } catch (_: Throwable) { /* native not yet initialized */ }
                delay(1_000)
            }
        }
    }

    private fun stopTelemetryBridge() {
        telemetryBridgeJob?.cancel()
        telemetryBridgeJob = null
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

        // Quick-send PTT action — toggles transmit directly from the shade so
        // the user never has to reopen the app. State is local to the
        // notification's tap cycle and reflected in the label/icon.
        val toggleIntent = Intent(ACTION_TOGGLE_PTT).setPackage(packageName)
        val togglePendingIntent = PendingIntent.getBroadcast(
            this, 2, toggleIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val toggleLabel = if (notificationPttActive) "Stop" else "Push to talk"
        builder.addAction(
            android.R.drawable.ic_btn_speak_now,
            toggleLabel,
            togglePendingIntent
        )

        // Legacy "Open PTT" action — kept behind the existing preference for
        // users who explicitly want the in-app PTT experience from lock screen.
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
