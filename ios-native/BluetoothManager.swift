// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-6HRT5KFHKTBK
//  BluetoothManager.swift
//  SassyTalkie iOS — Bluetooth peer-finding (CoreBluetooth)
//  Copyright 2025 Sassy Consulting LLC. All rights reserved.
//
//  WHAT THIS IS
//  ------------
//  On iOS, Bluetooth is the *peer-discovery* plane only. iOS does not expose
//  Bluetooth Classic / RFCOMM to third-party apps, so audio travels over the IP
//  transport (see the Rust `TransportManager`). This class advertises and scans for
//  the shared SassyTalkie BLE service UUID so iOS and Android devices find each
//  other, then reports discovered peers into the Rust core via the `sassytalkie_bt_*`
//  FFI. The Rust core owns the canonical roster (lib.rs / bluetooth.rs).
//
//  The service UUID MUST match Android `BleSignalingService.SERVICE_UUID` and the
//  Rust `SASSYTALKIE_BLE_SERVICE_UUID` constant, or cross-platform discovery breaks.
//
//  REQUIRED Info.plist KEYS
//  ------------------------
//    NSBluetoothAlwaysUsageDescription      = "SassyTalkie uses Bluetooth to find nearby radios."
//    NSBluetoothPeripheralUsageDescription  = "SassyTalkie advertises so nearby radios can find you."  (iOS < 13)
//  And, for background discovery, add the `bluetooth-central` and `bluetooth-peripheral`
//  UIBackgroundModes (note the iOS background-advertising caveat below).
//
//  FFI PREREQUISITE
//  ----------------
//  The `sassytalkie_bt_*` C functions must be visible to Swift. Regenerate the
//  bridging header with cbindgen (per cbindgen.toml) after adding the FFI block to
//  lib.rs, and import it from the app's bridging header / module map.
//
//  KNOWN iOS LIMITATIONS (documented, not bugs)
//  --------------------------------------------
//  • Background advertising: when the app is backgrounded, iOS moves the service
//    UUID into the advert "overflow" area, which another iOS device CANNOT match
//    with a service-filtered scan. Foreground↔foreground and iOS→Android (Android
//    scans the overflow area) discovery work; iOS-background→iOS is unreliable.
//  • There is no clean "peer lost" event for never-connected peripherals. We prune
//    stale peers on a timer (see `staleCheck`) mirroring the Android liveness model.

import Foundation
import CoreBluetooth

@objc public final class SassyBluetoothManager: NSObject {

    /// Must match Android `BleSignalingService.SERVICE_UUID` and Rust `SASSYTALKIE_BLE_SERVICE_UUID`.
    public static let serviceUUID = CBUUID(string: "b1a2e5d4-d5ab-7890-bede-fa12345678f0")

    /// Drop a discovered peer if we haven't seen an advert from it in this long.
    private static let staleAfter: TimeInterval = 12.0

    @objc public static let shared = SassyBluetoothManager()

    private var central: CBCentralManager!
    private var peripheral: CBPeripheralManager!
    private let queue = DispatchQueue(label: "com.sassyconsulting.sassytalkie.ble")

    // Strong refs so discovered peripherals survive until we (optionally) connect.
    private var discovered: [UUID: CBPeripheral] = [:]
    // Last-seen timestamps for staleness pruning (no native "lost" event in BLE scan).
    private var lastSeen: [UUID: Date] = [:]
    private var staleTimer: DispatchSourceTimer?

    private override init() {
        super.init()
        central = CBCentralManager(delegate: self, queue: queue)
        peripheral = CBPeripheralManager(delegate: self, queue: queue)
    }

    /// Begin advertising + scanning. Safe to call before the radios are powered on;
    /// the delegate `…DidUpdateState` callbacks kick the work off once ready.
    @objc public func start() {
        queue.async { [weak self] in
            guard let self = self else { return }
            if self.central.state == .poweredOn { self.startScanning() }
            if self.peripheral.state == .poweredOn { self.startAdvertising() }
            self.startStaleTimer()
        }
    }

    @objc public func stop() {
        queue.async { [weak self] in
            guard let self = self else { return }
            if self.central.isScanning { self.central.stopScan() }
            self.peripheral.stopAdvertising()
            self.staleTimer?.cancel()
            self.staleTimer = nil
            self.discovered.removeAll()
            self.lastSeen.removeAll()
            sassytalkie_bt_clear_connected()
        }
    }

    // MARK: - Private

    private func startScanning() {
        central.scanForPeripherals(
            withServices: [Self.serviceUUID],
            options: [CBCentralManagerScanOptionAllowDuplicatesKey: true] // refresh lastSeen
        )
        NSLog("SassyTalkie BLE: scanning for peers")
    }

    private func startAdvertising() {
        peripheral.startAdvertising([
            CBAdvertisementDataServiceUUIDsKey: [Self.serviceUUID]
        ])
        NSLog("SassyTalkie BLE: advertising service")
    }

    private func reportFound(id: String, name: String, rssi: Int32) {
        id.withCString { idC in
            name.withCString { nameC in
                _ = sassytalkie_bt_device_found(idC, nameC, rssi)
            }
        }
    }

    private func reportLost(id: String) {
        id.withCString { _ = sassytalkie_bt_device_lost($0) }
    }

    private func startStaleTimer() {
        staleTimer?.cancel()
        let t = DispatchSource.makeTimerSource(queue: queue)
        t.schedule(deadline: .now() + 3.0, repeating: 3.0)
        t.setEventHandler { [weak self] in self?.pruneStale() }
        staleTimer = t
        t.resume()
    }

    private func pruneStale() {
        let now = Date()
        let dead = lastSeen.filter { now.timeIntervalSince($0.value) > Self.staleAfter }.map { $0.key }
        for uuid in dead {
            lastSeen[uuid] = nil
            discovered[uuid] = nil
            reportLost(id: uuid.uuidString)
            NSLog("SassyTalkie BLE: peer %@ pruned (stale)", uuid.uuidString)
        }
    }
}

// MARK: - CBCentralManagerDelegate (scanning)

extension SassyBluetoothManager: CBCentralManagerDelegate {

    public func centralManagerDidUpdateState(_ central: CBCentralManager) {
        if central.state == .poweredOn {
            startScanning()
        } else {
            NSLog("SassyTalkie BLE: central state = %ld (not poweredOn)", central.state.rawValue)
        }
    }

    public func centralManager(_ central: CBCentralManager,
                               didDiscover peripheral: CBPeripheral,
                               advertisementData: [String: Any],
                               rssi RSSI: NSNumber) {
        let uuid = peripheral.identifier
        let isNew = (discovered[uuid] == nil)
        discovered[uuid] = peripheral
        lastSeen[uuid] = Date()

        // Only push into the Rust roster on first sight; duplicates just refresh lastSeen.
        guard isNew else { return }
        let name = peripheral.name
            ?? (advertisementData[CBAdvertisementDataLocalNameKey] as? String)
            ?? "SassyTalkie"
        reportFound(id: uuid.uuidString, name: name, rssi: RSSI.int32Value)
        NSLog("SassyTalkie BLE: discovered %@ (rssi=%@)", name, RSSI)
    }

    public func centralManager(_ central: CBCentralManager,
                               didDisconnectPeripheral peripheral: CBPeripheral,
                               error: Error?) {
        let uuid = peripheral.identifier
        discovered[uuid] = nil
        lastSeen[uuid] = nil
        reportLost(id: uuid.uuidString)
    }
}

// MARK: - CBPeripheralManagerDelegate (advertising)

extension SassyBluetoothManager: CBPeripheralManagerDelegate {

    public func peripheralManagerDidUpdateState(_ peripheral: CBPeripheralManager) {
        if peripheral.state == .poweredOn {
            startAdvertising()
        } else {
            NSLog("SassyTalkie BLE: peripheral state = %ld (not poweredOn)", peripheral.state.rawValue)
        }
    }
}
