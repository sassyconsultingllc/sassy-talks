package com.sassyconsulting.sassytalkie

import android.bluetooth.*
import android.bluetooth.le.*
import android.content.Context
import android.os.ParcelUuid
import android.util.Log
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean

private val CCCD_UUID: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")

/**
 * BLE GATT signaling channel for SassyTalkie.
 *
 * Architecture: BLE = control plane, RFCOMM = data plane.
 * Like real radios: squelch tone (BLE) opens carrier (RFCOMM).
 *
 * Commands sent/received over BLE GATT characteristics:
 * - PTT_START (0x01): Sender is about to transmit audio
 * - PTT_STOP (0x02): Sender stopped transmitting
 * - READY_ACK (0x03): Receiver acknowledges, RFCOMM ready
 * - PING (0x04): Keepalive
 * - CHANNEL_SYNC (0x05 + channel byte): Channel update
 */
@android.annotation.SuppressLint("MissingPermission")
class BleSignalingService(
    private val context: Context,
    private val adapter: BluetoothAdapter
) {

    companion object {
        private const val TAG = "BLE.Signal"

        val SERVICE_UUID: UUID = UUID.fromString("b1a2e5d4-d5ab-7890-bede-fa12345678f0")
        val PTT_CHAR_UUID: UUID = UUID.fromString("b1a2e5d4-d5ab-7890-bede-fa12345678f1")
        val CHANNEL_CHAR_UUID: UUID = UUID.fromString("b1a2e5d4-d5ab-7890-bede-fa12345678f2")

        const val CMD_PTT_START: Byte = 0x01
        const val CMD_PTT_STOP: Byte = 0x02
        const val CMD_READY_ACK: Byte = 0x03
        const val CMD_PING: Byte = 0x04
        const val CMD_CHANNEL_SYNC: Byte = 0x05
    }

    interface Listener {
        fun onPttStartReceived(deviceAddress: String)
        fun onPttStopReceived(deviceAddress: String)
        fun onReadyAckReceived(deviceAddress: String)
        fun onPeerDiscovered(device: BluetoothDevice)
        fun onPeerLost(deviceAddress: String)
    }

    var listener: Listener? = null

    private val advertising = AtomicBoolean(false)
    private val scanning = AtomicBoolean(false)
    private var gattServer: BluetoothGattServer? = null
    private var advertiser: BluetoothLeAdvertiser? = null
    private var scanner: BluetoothLeScanner? = null
    private val connectedPeers = ConcurrentHashMap<String, BluetoothDevice>()
    private val peerGattClients = ConcurrentHashMap<String, BluetoothGatt>()

    // —— GATT Server (receives commands from peers) ——

    fun startServer() {
        val manager = context.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager ?: return

        val pttChar = BluetoothGattCharacteristic(
            PTT_CHAR_UUID,
            BluetoothGattCharacteristic.PROPERTY_WRITE or BluetoothGattCharacteristic.PROPERTY_NOTIFY,
            BluetoothGattCharacteristic.PERMISSION_WRITE
        )
        // Add CCCD descriptor so clients can subscribe to notifications
        val cccd = BluetoothGattDescriptor(
            CCCD_UUID,
            BluetoothGattDescriptor.PERMISSION_READ or BluetoothGattDescriptor.PERMISSION_WRITE
        )
        pttChar.addDescriptor(cccd)

        val channelChar = BluetoothGattCharacteristic(
            CHANNEL_CHAR_UUID,
            BluetoothGattCharacteristic.PROPERTY_READ or BluetoothGattCharacteristic.PROPERTY_WRITE,
            BluetoothGattCharacteristic.PERMISSION_READ or BluetoothGattCharacteristic.PERMISSION_WRITE
        )

        val service = BluetoothGattService(SERVICE_UUID, BluetoothGattService.SERVICE_TYPE_PRIMARY)
        service.addCharacteristic(pttChar)
        service.addCharacteristic(channelChar)

        gattServer = manager.openGattServer(context, gattServerCallback)
        gattServer?.addService(service)

        Log.i(TAG, "GATT server started")
    }

    fun stopServer() {
        gattServer?.close()
        gattServer = null
        Log.i(TAG, "GATT server stopped")
    }

    private val gattServerCallback = object : BluetoothGattServerCallback() {

        override fun onConnectionStateChange(device: BluetoothDevice, status: Int, newState: Int) {
            if (newState == BluetoothProfile.STATE_CONNECTED) {
                connectedPeers[device.address] = device

                Log.i(TAG, "BLE peer connected: ${device.name ?: device.address}")
                listener?.onPeerDiscovered(device)
            } else if (newState == BluetoothProfile.STATE_DISCONNECTED) {
                connectedPeers.remove(device.address)
                listener?.onPeerLost(device.address)

                Log.i(TAG, "BLE peer disconnected: ${device.address}")
            }
        }

        override fun onCharacteristicWriteRequest(
            device: BluetoothDevice, requestId: Int,
            characteristic: BluetoothGattCharacteristic,
            preparedWrite: Boolean, responseNeeded: Boolean,
            offset: Int, value: ByteArray?
        ) {
            if (characteristic.uuid == PTT_CHAR_UUID && value != null && value.isNotEmpty()) {
                when (value[0]) {
                    CMD_PTT_START -> {
                        Log.i(TAG, "\u2190 PTT_START from ${device.name ?: device.address}")
                        listener?.onPttStartReceived(device.address)
                    }
                    CMD_PTT_STOP -> {
                        Log.i(TAG, "\u2190 PTT_STOP from ${device.name ?: device.address}")
                        listener?.onPttStopReceived(device.address)
                    }
                    CMD_READY_ACK -> {
                        Log.i(TAG, "\u2190 READY_ACK from ${device.name ?: device.address}")
                        listener?.onReadyAckReceived(device.address)
                    }
                }
            }

            if (responseNeeded) {
                gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, 0, null)
            }
        }

        override fun onDescriptorWriteRequest(
            device: BluetoothDevice, requestId: Int,
            descriptor: BluetoothGattDescriptor,
            preparedWrite: Boolean, responseNeeded: Boolean,
            offset: Int, value: ByteArray?
        ) {
            if (descriptor.uuid == CCCD_UUID) {
                descriptor.value = value
                Log.d(TAG, "CCCD write from ${device.name ?: device.address}")
            }
            if (responseNeeded) {
                gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, 0, null)
            }
        }
    }

    // —— BLE Advertising (makes us discoverable) ——

    fun startAdvertising() {
        advertiser = adapter.bluetoothLeAdvertiser
        if (advertiser == null) {
            Log.w(TAG, "BLE advertising not supported on this device")
            return
        }

        val settings = AdvertiseSettings.Builder()
            .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_LATENCY)
            .setConnectable(true)
            .setTimeout(0) // Advertise indefinitely
            .build()

        val data = AdvertiseData.Builder()
            .setIncludeDeviceName(true)
            .addServiceUuid(ParcelUuid(SERVICE_UUID))
            .build()

        advertiser?.startAdvertising(settings, data, advertiseCallback)
        Log.i(TAG, "BLE advertising requested")
    }

    fun stopAdvertising() {
        if (advertising.getAndSet(false)) {
            advertiser?.stopAdvertising(advertiseCallback)
            Log.i(TAG, "BLE advertising stopped")
        }
    }

    private val advertiseCallback = object : AdvertiseCallback() {
        override fun onStartSuccess(settingsInEffect: AdvertiseSettings?) {
            advertising.set(true)
            Log.i(TAG, "BLE advertising active")
        }

        override fun onStartFailure(errorCode: Int) {
            Log.e(TAG, "BLE advertising failed: error $errorCode")
            advertising.set(false)
        }
    }

    // —— BLE Scanning (discovers peers) ——

    fun startScanning() {
        scanner = adapter.bluetoothLeScanner
        if (scanner == null) {
            Log.w(TAG, "BLE scanner not available")
            return
        }

        val filter = ScanFilter.Builder()
            .setServiceUuid(ParcelUuid(SERVICE_UUID))
            .build()

        val settings = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
            .build()

        scanner?.startScan(listOf(filter), settings, scanCallback)
        scanning.set(true)
        Log.i(TAG, "BLE scanning started")
    }

    fun stopScanning() {
        if (scanning.getAndSet(false)) {
            scanner?.stopScan(scanCallback)
            Log.i(TAG, "BLE scanning stopped")
        }
    }

    private val scanCallback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, result: ScanResult) {
            val device = result.device
            if (!connectedPeers.containsKey(device.address)) {
                Log.i(TAG, "Discovered SassyTalkie peer: ${device.name ?: device.address}")
                listener?.onPeerDiscovered(device)
            }
        }
    }

    // —— Send Commands to Peers ——

    fun broadcastPttStart() {
        broadcastCommand(CMD_PTT_START, "PTT_START")
    }

    fun broadcastPttStop() {
        broadcastCommand(CMD_PTT_STOP, "PTT_STOP")
    }

    fun sendReadyAck(deviceAddress: String) {
        sendCommandToPeer(deviceAddress, CMD_READY_ACK, "READY_ACK")
    }

    private fun broadcastCommand(cmd: Byte, label: String) {
        val count = peerGattClients.size
        Log.i(TAG, "\u2192 Broadcasting $label to $count BLE peers")
        for ((address, gatt) in peerGattClients) {
            writeCommand(gatt, cmd, address, label)
        }
    }

    private fun sendCommandToPeer(address: String, cmd: Byte, label: String) {
        val gatt = peerGattClients[address]
        if (gatt != null) {
            writeCommand(gatt, cmd, address, label)
        } else {
            Log.w(TAG, "No GATT client for $address, cannot send $label")
        }
    }

    private fun writeCommand(gatt: BluetoothGatt, cmd: Byte, address: String, label: String) {
        val service = gatt.getService(SERVICE_UUID)
        val char = service?.getCharacteristic(PTT_CHAR_UUID)
        if (char != null) {
            char.value = byteArrayOf(cmd)
            val success = gatt.writeCharacteristic(char)
            Log.d(TAG, "\u2192 $label to $address (success=$success)")
        }
    }

    // —— Connect to a discovered peer's GATT server ——

    fun connectToPeer(device: BluetoothDevice) {
        if (peerGattClients.containsKey(device.address)) return

        Log.i(TAG, "Connecting GATT client to ${device.name ?: device.address}")
        device.connectGatt(context, false, object : BluetoothGattCallback() {

            override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
                if (newState == BluetoothProfile.STATE_CONNECTED) {
                    peerGattClients[device.address] = gatt
                    connectedPeers[device.address] = device
                    gatt.discoverServices()

                    Log.i(TAG, "GATT client connected to ${device.name ?: device.address}")
                } else if (newState == BluetoothProfile.STATE_DISCONNECTED) {
                    peerGattClients.remove(device.address)
                    connectedPeers.remove(device.address)
                    gatt.close()

                    listener?.onPeerLost(device.address)
                    Log.i(TAG, "GATT client disconnected from ${device.address}")
                }
            }

            override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
                if (status == BluetoothGatt.GATT_SUCCESS) {
                    val svc = gatt.getService(SERVICE_UUID)
                    if (svc != null) {
                        Log.i(TAG, "BLE signaling channel ready with ${device.name ?: device.address}")

                        // Enable notifications on PTT characteristic
                        val pttChar = svc.getCharacteristic(PTT_CHAR_UUID)
                        if (pttChar != null) {
                            gatt.setCharacteristicNotification(pttChar, true)
                            // Write CCCD descriptor to actually enable remote notifications
                            val desc = pttChar.getDescriptor(CCCD_UUID)
                            if (desc != null) {
                                desc.value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
                                gatt.writeDescriptor(desc)
                            }
                        }
                    }
                }
            }

            override fun onCharacteristicChanged(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic) {
                if (characteristic.uuid == PTT_CHAR_UUID) {
                    val value = characteristic.value
                    if (value != null && value.isNotEmpty()) {
                        when (value[0]) {
                            CMD_PTT_START -> listener?.onPttStartReceived(device.address)
                            CMD_PTT_STOP -> listener?.onPttStopReceived(device.address)
                            CMD_READY_ACK -> listener?.onReadyAckReceived(device.address)
                        }
                    }
                }
            }
        })
    }

    // —— Lifecycle ——

    val blePeerCount: Int get() = connectedPeers.size
    val blePeers: List<BluetoothDevice> get() = connectedPeers.values.toList()

    fun shutdown() {
        stopAdvertising()
        stopScanning()

        for ((_, gatt) in peerGattClients) {
            gatt.close()
        }

        peerGattClients.clear()
        connectedPeers.clear()
        stopServer()
    }
}
