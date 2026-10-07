// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-GDROUYFZ4F2D
//
//  RelayClient.swift
//  SassyTalkie — Cloudflare relay WebSocket client (remote peers).
//  Copyright © 2025 Sassy Consulting LLC. All rights reserved.
//
//  The socket lives here (URLSessionWebSocketTask); the Rust core supplies the
//  room id, seals/opens audio frames, and builds heartbeats over the C FFI —
//  mirroring the android-native queue-bridge model so the relay wire bytes are
//  byte-identical to the phone (and the desktop, verified in tauri-desktop).
//
//  Wire protocol (must match android-native / tauri-desktop):
//    1. GET  https://relay.sassyconsultingllc.com/auth?room=<room>&peer=<peer>
//         → {"token": "..."}
//    2. wss://relay.sassyconsultingllc.com/ws?room=&device=&peer=&client_id=
//         X-Sassy-Token: <token>   (+ since=<ms> after a short drop)
//    3. binary WS messages = sealed core::wire frames (+ sealed OP_HEARTBEAT
//       ~every 2 s). Relay catch-up frames are unwrapped in Rust.
//
//  Threading: every piece of mutable state is confined to `queue`. URLSession
//  and NWPathMonitor callbacks hop onto it; nothing here touches the main
//  thread, so audio TX never waits on UI work.

import Foundation
import Network
import UIKit

final class RelayClient {

    private static let httpsBase = "https://relay.sassyconsultingllc.com"
    private static let wssBase = "wss://relay.sassyconsultingllc.com"
    private static let heartbeatInterval: DispatchTimeInterval = .seconds(2)
    private static let drainInterval: DispatchTimeInterval = .milliseconds(10)
    /// Reconnect backoff: 1, 2, 4, 8, 16, then 30 s forever, with ±20 % jitter.
    private static let maxBackoff: Double = 30

    private let queue = DispatchQueue(label: "com.sassyconsulting.sassytalkie.relay", qos: .userInitiated)
    private let session = PinnedURLSession.shared
    private let pathMonitor = NWPathMonitor()

    // ── state below is only touched on `queue` ──
    private var task: URLSessionWebSocketTask?
    private var heartbeatTimer: DispatchSourceTimer?
    private var drainTimer: DispatchSourceTimer?
    private var reconnectWork: DispatchWorkItem?
    /// The user wants the relay up (paired + not wiped).
    private var wantConnected = false
    /// Bumped on every dial/teardown; callbacks from an older socket are ignored.
    private var generation = 0
    /// The current socket has received the relay's welcome (handshake done).
    private var isLive = false
    private var attempts = 0
    /// After the first live socket, reconnects ask the relay for the gap.
    private var hasCompletedHandshake = false
    /// Unix ms the socket was last known alive (welcome or any inbound frame).
    private var lastAliveMs: UInt64 = 0
    private var networkSatisfied = true

    /// Same stable peer id PresenceClient uses so /presence and WS identity match.
    private var peerId: String { PresenceClient.peerId }
    private let deviceName = UIDevice.current.name

    init() {
        pathMonitor.pathUpdateHandler = { [weak self] path in
            self?.queue.async { self?.networkChanged(satisfied: path.status == .satisfied) }
        }
        pathMonitor.start(queue: queue)
    }

    deinit {
        pathMonitor.cancel()
    }

    /// Connect to the relay for the current paired session. No-op if unpaired
    /// or already up.
    func connect() {
        queue.async {
            guard !self.wantConnected else { return }
            self.wantConnected = true
            self.attempts = 0
            self.dial()
        }
    }

    /// Warm reconnect after a wake push — tear down and dial again (with a
    /// catch-up cursor if the last socket was alive recently).
    func reconnectForWake() {
        queue.async {
            self.wantConnected = true
            self.attempts = 0
            self.teardownSocket()
            self.dial()
        }
    }

    func disconnect() {
        queue.async {
            self.wantConnected = false
            self.teardownSocket()
        }
    }

    // MARK: - Dial

    private static func relayRoomId() -> String? {
        guard let c = sassytalkie_relay_room_id() else { return nil }
        defer { sassytalkie_free_string(c) }
        let s = String(cString: c)
        return s.isEmpty ? nil : s
    }

    private static func nowMs() -> UInt64 {
        UInt64(Date().timeIntervalSince1970 * 1000)
    }

    /// RFC 3986 unreserved set only — what Android and desktop encode with.
    /// `.urlQueryAllowed` leaves `&`, `=`, `+` and `#` alone, so a device name
    /// like "Ann & Bo's iPad" split the query and a `+` read back as a space.
    private static func enc(_ s: String) -> String {
        var allowed = CharacterSet.alphanumerics
        allowed.insert(charactersIn: "-._~")
        return s.addingPercentEncoding(withAllowedCharacters: allowed) ?? ""
    }

    private func dial() {
        guard wantConnected else { return }
        guard let room = Self.relayRoomId() else {
            print("Relay: not paired (no room id) — not connecting")
            wantConnected = false
            return
        }
        generation += 1
        let gen = generation
        fetchToken(room: room) { [weak self] token in
            guard let self = self else { return }
            self.queue.async {
                guard gen == self.generation, self.wantConnected else { return }
                guard let token = token else {
                    // Used to give up here for good: one failed /auth during a
                    // network blip left the radio permanently offline.
                    print("Relay: token fetch failed — will retry")
                    self.scheduleReconnect()
                    return
                }
                self.openSocket(room: room, token: token, gen: gen)
                PresenceClient.uploadCurrentToken(roomId: room)
            }
        }
    }

    private func fetchToken(room: String, completion: @escaping (String?) -> Void) {
        guard let url = URL(string: "\(Self.httpsBase)/auth?room=\(Self.enc(room))&peer=\(Self.enc(peerId))") else {
            completion(nil)
            return
        }
        var req = URLRequest(url: url)
        req.timeoutInterval = 10
        session.dataTask(with: req) { data, response, _ in
            guard (response as? HTTPURLResponse).map({ (200..<300).contains($0.statusCode) }) == true,
                  let data = data,
                  let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let token = json["token"] as? String, !token.isEmpty else {
                completion(nil)
                return
            }
            completion(token)
        }.resume()
    }

    private func openSocket(room: String, token: String, gen: Int) {
        var urlStr = "\(Self.wssBase)/ws?room=\(Self.enc(room))"
            + "&device=\(Self.enc(deviceName))&peer=\(Self.enc(peerId))"
            + "&client_id=\(Self.enc(UUID().uuidString))"
        if hasCompletedHandshake {
            let since = sassytalkie_relay_catchup_since(lastAliveMs, Self.nowMs())
            if since > 0 { urlStr += "&since=\(since)" }
        }
        guard let url = URL(string: urlStr) else {
            scheduleReconnect()
            return
        }
        // Token in a header, not the query string, so it stays out of access
        // logs. X-Sassy-Token rather than Authorization: Apple lists
        // Authorization among the headers URLSession may manage itself. The
        // relay accepts both (relay-auth extractToken; verified live).
        var req = URLRequest(url: url)
        req.setValue(token, forHTTPHeaderField: "X-Sassy-Token")

        let task = session.webSocketTask(with: req)
        self.task = task
        isLive = false
        task.resume()
        receiveLoop(task: task, gen: gen)
        startTimers(gen: gen)
    }

    // MARK: - IO

    private func receiveLoop(task: URLSessionWebSocketTask, gen: Int) {
        task.receive { [weak self] result in
            guard let self = self else { return }
            self.queue.async {
                guard gen == self.generation, self.wantConnected else { return }
                switch result {
                case .failure(let err):
                    print("Relay: receive error \(err.localizedDescription) — reconnecting")
                    self.teardownSocket()
                    self.scheduleReconnect()
                case .success(let message):
                    switch message {
                    case .string(let text):
                        if text.contains("\"welcome\"") { self.markLive() }
                        self.lastAliveMs = Self.nowMs()
                    case .data(let data):
                        self.lastAliveMs = Self.nowMs()
                        if !self.isLive { self.markLive() }
                        guard !data.isEmpty else { break }
                        data.withUnsafeBytes { (raw: UnsafeRawBufferPointer) in
                            if let base = raw.bindMemory(to: UInt8.self).baseAddress {
                                _ = sassytalkie_relay_on_message(base, data.count)
                            }
                        }
                    @unknown default:
                        break
                    }
                    self.receiveLoop(task: task, gen: gen)
                }
            }
        }
    }

    private func markLive() {
        guard !isLive else { return }
        isLive = true
        hasCompletedHandshake = true
        lastAliveMs = Self.nowMs()
        attempts = 0
        // Only tee TX into the relay queue once the socket is really open, so a
        // failed dial does not buffer stale audio to flush later.
        sassytalkie_relay_set_active(true)
        print("Relay: live")
    }

    private func startTimers(gen: Int) {
        stopTimers()
        let hb = DispatchSource.makeTimerSource(queue: queue)
        hb.schedule(deadline: .now() + Self.heartbeatInterval, repeating: Self.heartbeatInterval)
        hb.setEventHandler { [weak self] in
            guard let self = self, gen == self.generation else { return }
            self.sendHeartbeat()
        }
        hb.resume()
        heartbeatTimer = hb

        let drain = DispatchSource.makeTimerSource(queue: queue)
        drain.schedule(deadline: .now() + Self.drainInterval, repeating: Self.drainInterval,
                       leeway: .milliseconds(2))
        drain.setEventHandler { [weak self] in
            guard let self = self, gen == self.generation, self.isLive else { return }
            self.drainOutbound()
        }
        drain.resume()
        drainTimer = drain
    }

    private func stopTimers() {
        heartbeatTimer?.cancel(); heartbeatTimer = nil
        drainTimer?.cancel(); drainTimer = nil
    }

    /// Drain every sealed frame queued by the Rust TX path this tick and send it.
    private func drainOutbound() {
        guard let task = task else { return }
        var len = 0
        while let ptr = sassytalkie_relay_poll_outbound(&len), len > 0 {
            let data = Data(bytes: ptr, count: len)
            sassytalkie_free_bytes(ptr, len)
            task.send(.data(data)) { err in
                if let err = err { print("Relay: send error \(err.localizedDescription)") }
            }
        }
    }

    private func sendHeartbeat() {
        guard let task = task else { return }
        var len = 0
        guard let ptr = sassytalkie_relay_heartbeat_frame(&len) else { return }
        guard len > 0 else { return }
        let data = Data(bytes: ptr, count: len)
        sassytalkie_free_bytes(ptr, len)
        task.send(.data(data)) { _ in }
    }

    // MARK: - Lifecycle

    private func teardownSocket() {
        generation += 1
        reconnectWork?.cancel(); reconnectWork = nil
        stopTimers()
        if isLive || task != nil {
            sassytalkie_relay_set_active(false)
        }
        isLive = false
        task?.cancel(with: .goingAway, reason: nil)
        task = nil
    }

    private func scheduleReconnect() {
        guard wantConnected else { return }
        reconnectWork?.cancel()
        // Offline: wait for NWPathMonitor to report a usable path instead of
        // burning attempts (and battery) against a dead radio.
        guard networkSatisfied else { return }
        let base = min(Self.maxBackoff, pow(2, Double(attempts)))
        let delay = base * Double.random(in: 0.8...1.2)
        attempts += 1
        let work = DispatchWorkItem { [weak self] in
            guard let self = self, self.wantConnected else { return }
            self.teardownSocket()
            self.dial()
        }
        reconnectWork = work
        queue.asyncAfter(deadline: .now() + delay, execute: work)
    }

    private func networkChanged(satisfied: Bool) {
        let regained = satisfied && !networkSatisfied
        networkSatisfied = satisfied
        guard wantConnected else { return }
        if regained && !isLive {
            // Path came back (Wi-Fi ↔ cellular, tunnel exit): dial now rather
            // than waiting out the current backoff.
            attempts = 0
            teardownSocket()
            dial()
        }
    }
}
