// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-S5YHEELE2MAH
//
//  SassyTalkieViewModel.swift
//  SassyTalkie
//
//  Copyright © 2025 Sassy Consulting LLC. All rights reserved.
//

import Foundation
import SwiftUI
import Combine

/// View model for SassyTalkie app
class SassyTalkieViewModel: ObservableObject {
    
    // MARK: - Published Properties
    
    @Published var channel: UInt8 = 1
    @Published var isPTTPressed: Bool = false
    @Published var isTransmitting: Bool = false
    @Published var isReceiving: Bool = false
    @Published var isConnected: Bool = false
    @Published var statusText: String = "Initializing..."
    @Published var showingSettings: Bool = false
    @Published var showingScanner: Bool = false
    @Published var showingHostQR: Bool = false
    /// The QR JSON to render when hosting a channel (nil unless hosting).
    @Published var hostQRJSON: String? = nil
    /// True once a QR session is installed — audio is encrypted (mandatory) and
    /// cross-platform on the paired channel. Until then TX refuses to send.
    @Published var isPaired: Bool = false
    @Published var isEntitled: Bool = StoreKitEntitlements.isUnlockedCached
    @Published var showingPaywall: Bool = false
    @Published var pttRejectText: String? = nil
    @Published var trialWarning: String? = nil

    /// Until when a transient message (pairing result, invite error, wake)
    /// owns the status line. The 100 ms state poll used to overwrite every
    /// message on its next tick, so users never saw "Invite expired" etc.
    private var messageUntil = Date.distantPast

    /// Show `text` in the status line for `seconds` before radio state resumes.
    /// Main thread only.
    private func flash(_ text: String, seconds: TimeInterval = 4) {
        statusText = text
        messageUntil = Date().addingTimeInterval(seconds)
    }

    /// Highest channel a session supports (core MAX_CHANNELS).
    let maxChannel: UInt8 = sassytalkie_max_channel()
    
    var version: String {
        let cString = sassytalkie_get_version()
        let version = String(cString: cString)
        sassytalkie_free_string(UnsafeMutablePointer(mutating: cString))
        return version
    }
    
    var statusColor: Color {
        // Design tokens (Theme.swift). Also iOS-14-safe: .cyan/.teal are iOS 15+.
        if isTransmitting {
            return .stCoral
        } else if isReceiving {
            return .stTeal
        } else if isConnected {
            return .stOnline
        } else {
            return .stTextMuted
        }
    }
    
    // MARK: - Crypto / PQC (parity with Android SassyTalkNative)

    /// Install the QR pre-shared key (base64). Enables AES-256-GCM on the audio
    /// path (TX encrypts, RX decrypts + replay-checks).
    func setPsk(_ keyB64: String) -> Bool {
        return keyB64.withCString { sassytalkie_set_psk($0) }
    }

    /// Import a scanned QR session (the host's QR JSON). Switches to the QR's
    /// channel and installs its key using the SAME validation as Android, so the
    /// pairing is genuinely cross-platform. Returns the channel (1-8) on success,
    /// 0 on a malformed/expired QR. After success, audio is AES-256-GCM encrypted
    /// both ways and an Android peer on that channel can hear this device.
    @discardableResult
    func importSessionQR(_ json: String) -> Int {
        let ch = json.withCString { Int(sassytalkie_import_session_qr($0)) }
        if ch > 0 {
            KeychainStore.saveSessionQR(json)
            DispatchQueue.main.async {
                self.channel = UInt8(ch)
                self.isPaired = true
                self.showingScanner = false
                self.flash("Paired · ch \(ch)")
                // Bring up the relay for remote peers (room id was just set by
                // the import). Tear down any previous room first.
                self.relayClient.disconnect()
                self.relayClient.connect()
            }
        }
        return ch
    }

    // MARK: - Invite-link import (parity with Android SessionShareLink)

    private static let relayHost = "relay.sassyconsultingllc.com"
    private static let relayBase = "https://relay.sassyconsultingllc.com"

    /// Import an encrypted session invite from a tapped Universal Link
    /// `https://relay.sassyconsultingllc.com/v/<id>#<base64url-key>`: fetch the
    /// opaque blob from `/share/<id>`, decrypt it through the SHARED Rust core
    /// (`sassytalkie_decrypt_share_blob`), then import it exactly like a scanned
    /// QR. The decryption key rides only in the URL fragment and is never sent to
    /// the relay — the worker stores ciphertext it cannot read.
    func importFromShareURL(_ url: URL) {
        guard let comps = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
            DispatchQueue.main.async { self.flash("Not a SassyTalk invite link") }
            return
        }
        let isHttps = comps.scheme == "https" && comps.host == Self.relayHost && comps.path.hasPrefix("/v/")
        let isApp = ShareLinkClient.isAppScheme(comps.scheme) && comps.host == "v"
        guard isHttps || isApp else {
            DispatchQueue.main.async { self.flash("Not a SassyTalk invite link") }
            return
        }
        let id = String(comps.path.dropFirst(isApp ? 1 : "/v/".count))
        guard Self.isValidShareID(id) else {
            DispatchQueue.main.async { self.flash("Malformed invite link") }
            return
        }
        // base64url has no percent-escapes, so the (already percent-decoded)
        // fragment is the key verbatim.
        guard let key = comps.fragment, !key.isEmpty else {
            DispatchQueue.main.async { self.flash("Invite link is missing its key") }
            return
        }
        guard let fetchURL = URL(string: "\(Self.relayBase)/share/\(id)") else { return }

        DispatchQueue.main.async { self.flash("Opening invite…") }
        PinnedURLSession.shared.dataTask(with: fetchURL) { [weak self] data, response, error in
            guard let self = self else { return }
            if let error = error {
                DispatchQueue.main.async { self.flash("Network error: \(error.localizedDescription)") }
                return
            }
            let code = (response as? HTTPURLResponse)?.statusCode ?? 0
            if code == 404 { DispatchQueue.main.async { self.flash("Invite already used or expired") }; return }
            if code == 429 { DispatchQueue.main.async { self.flash("Too many requests; try later") }; return }
            guard (200..<300).contains(code), let blob = data, !blob.isEmpty else {
                DispatchQueue.main.async { self.flash("Server returned HTTP \(code)") }
                return
            }
            // Decrypt via the shared core (same accept/reject as Android & desktop).
            let json: String? = blob.withUnsafeBytes { raw -> String? in
                guard let base = raw.bindMemory(to: UInt8.self).baseAddress else { return nil }
                guard let cstr = key.withCString({ keyPtr in
                    sassytalkie_decrypt_share_blob(base, blob.count, keyPtr)
                }) else { return nil }
                defer { sassytalkie_free_string(cstr) }
                return String(cString: cstr)
            }
            guard let sessionJSON = json, !sessionJSON.isEmpty else {
                DispatchQueue.main.async { self.flash("Couldn't decrypt invite (link wrong or expired)") }
                return
            }
            // importSessionQR hops to main, flips isPaired, and connects the relay.
            if self.importSessionQR(sessionJSON) == 0 {
                DispatchQueue.main.async { self.flash("Invite session was invalid") }
            }
        }.resume()
    }

    /// The worker's share-id alphabet/length (share.js ID_RE): base64url, 16–64.
    private static func isValidShareID(_ id: String) -> Bool {
        let len = id.count
        guard len >= 16, len <= 64 else { return false }
        return id.allSatisfy { c in
            (c.isASCII && (c.isLetter || c.isNumber)) || c == "_" || c == "-"
        }
    }

    /// Host the current channel: mint a fresh QR (installs our own key so the host
    /// is paired too) and publish the JSON to render for a joiner to scan. The QR
    /// is cross-platform — an Android device can scan it to join the same channel.
    func hostChannel(durationHours: UInt32 = 24, groupName: String = "") {
        let ch = channel
        let json: String? = groupName.withCString { gp in
            guard let c = sassytalkie_generate_session_qr(ch, durationHours, gp) else { return nil }
            defer { sassytalkie_free_string(c) }
            return String(cString: c)
        }
        if let json = json {
            KeychainStore.saveSessionQR(json)
            DispatchQueue.main.async {
                self.isPaired = true
                self.hostQRJSON = json
                self.showingHostQR = true
                self.flash("Hosting · ch \(ch)")
                self.relayClient.disconnect()
                self.relayClient.connect()
            }
        }
    }

    /// This build's capability bitmap (hybrid-PQC support).
    func localCapabilities() -> UInt8 {
        return sassytalkie_local_capabilities()
    }

    /// Begin a classical X25519 key exchange. Returns our base64 public key, or nil.
    func keyExchangeInit() -> String? {
        guard let c = sassytalkie_key_exchange_init() else { return nil }
        defer { sassytalkie_free_string(c) }
        return String(cString: c)
    }

    /// Complete the classical key exchange with the peer's base64 public key.
    func keyExchangeComplete(_ remoteB64: String) -> Bool {
        return remoteB64.withCString { sassytalkie_key_exchange_complete($0) }
    }

    /// Initiator: begin a path-(a) hybrid PQC handshake. Returns the base64
    /// initiator message to send to the peer, or nil if no PSK is installed.
    func hybridHandshakeInit() -> String? {
        guard let c = sassytalkie_hybrid_handshake_init() else { return nil }
        defer { sassytalkie_free_string(c) }
        return String(cString: c)
    }

    /// Responder: install the session from the peer's base64 initiator message
    /// and return the base64 responder message to send back, or nil on failure.
    func hybridHandshakeRespond(_ initB64: String) -> String? {
        return initB64.withCString { initPtr -> String? in
            guard let c = sassytalkie_hybrid_handshake_respond(initPtr) else { return nil }
            defer { sassytalkie_free_string(c) }
            return String(cString: c)
        }
    }

    /// Initiator: complete with the peer's base64 reply, installing the PQ session.
    func hybridHandshakeComplete(_ respB64: String) -> Bool {
        return respB64.withCString { sassytalkie_hybrid_handshake_complete($0) }
    }

    func hybridHandshakeConfirm() -> Bool {
        return sassytalkie_hybrid_handshake_confirm()
    }

    // MARK: - Private Properties

    private let audioManager = AudioManager()
    /// Cloudflare relay client for remote peers. Auto-joins on pairing (mirrors
    /// Android's AutoConnectManager bringing the relay up alongside WiFi).
    private let relayClient = RelayClient()
    private var stateTimer: Timer?
    private var wakeObserver: NSObjectProtocol?
    
    // MARK: - Initialization
    
    init() {
        // Initialize Rust with this install's stable identity. The wire sender
        // id used to be random per launch, so Android saw a new radio after
        // every relaunch and floor tie-breaks compared a moving id.
        let success = PresenceClient.peerId.withCString { idPtr in
            UIDevice.current.name.withCString { namePtr in
                sassytalkie_init_with_identity(idPtr, namePtr)
            }
        }
        if success {
            print("✅ SassyTalkie initialized")
            statusText = "Ready"
            UpdateReset.runIfNeeded()
            ManagedConfig.apply()
            
            // Start listening
            _ = sassytalkie_start_listening()
            isConnected = true
            statusText = "Listening"
            
            // Start state polling
            startStatePolling()
            SassyBluetoothManager.shared.start()
            observeWakePushes()
            StoreKitEntitlements.startObservingTransactions { [weak self] ok in
                self?.isEntitled = ok
            }
            StoreKitEntitlements.refresh { [weak self] ok in
                DispatchQueue.main.async { self?.isEntitled = ok }
            }
            if ManagedConfig.forceSessionWipe {
                wipeSession(source: "managed_restriction")
            } else if let stored = KeychainStore.loadSessionQR(), importSessionQR(stored) > 0 {
                print("Restored session from Keychain")
            }
        } else {
            print("❌ Failed to initialize SassyTalkie")
            statusText = "Error"
        }
    }

    private func observeWakePushes() {
        wakeObserver = NotificationCenter.default.addObserver(
            forName: .sassyTalkieWake,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            guard let self = self else { return }
            // Warm path: reconnect relay (catchup=1 after first handshake).
            // Cold path still requires the user to have opened the app via the
            // notification — mic cannot start from a silent push.
            self.flash("Wake — reconnecting…")
            self.relayClient.reconnectForWake()
        }
    }
    
    func wipeSession(source: String = "user") {
        if let room = Self.currentRoomId() {
            DispatchQueue.global(qos: .utility).async {
                _ = PresenceClient.remove(roomId: room)
            }
        }
        sassytalkie_wipe_session()
        KeychainStore.deleteSession()
        relayClient.disconnect()
        DispatchQueue.main.async {
            self.isPaired = false
            self.hostQRJSON = nil
            self.flash(source == "managed_restriction"
                ? "Session cleared (MDM)"
                : "Session cleared")
        }
    }

    /// Technical audit export — not a legal chain of custody / not court-certified evidence.
    func exportAuditJson() -> String? {
        guard let c = sassytalkie_export_audit() else { return nil }
        defer { sassytalkie_free_string(c) }
        return String(cString: c)
    }

    deinit {
        if let wakeObserver {
            NotificationCenter.default.removeObserver(wakeObserver)
        }
        stateTimer?.invalidate()
        relayClient.disconnect()
        SassyBluetoothManager.shared.stop()
        sassytalkie_shutdown()
    }
    
    // MARK: - Channel Control
    
    func incrementChannel() {
        if channel < maxChannel {
            channel += 1
            _ = sassytalkie_set_channel(channel)
        }
    }
    
    func decrementChannel() {
        if channel > 1 {
            channel -= 1
            _ = sassytalkie_set_channel(channel)
        }
    }
    
    // MARK: - PTT Control
    
    func pttPress() {
        guard !isPTTPressed else { return }
        guard TrialStore.mayUseRadio(entitled: isEntitled) else {
            showingPaywall = true
            return
        }

        isPTTPressed = true

        let success = sassytalkie_ptt_press()
        if success {
            do {
                try audioManager.startRecording()
                print("🎤 PTT pressed")
            } catch {
                print("❌ Failed to start recording: \(error)")
                isPTTPressed = false
                _ = sassytalkie_ptt_release()
            }
        } else {
            if let c = sassytalkie_take_ptt_reject() {
                defer { sassytalkie_free_string(c) }
                pttRejectText = String(cString: c)
            } else {
                pttRejectText = "Couldn't start PTT"
            }
            print("❌ Failed to start PTT")
            isPTTPressed = false
        }
    }
    
    func pttRelease() {
        guard isPTTPressed else { return }
        
        isPTTPressed = false
        audioManager.stopRecording()
        _ = sassytalkie_ptt_release()
        print("🎤 PTT released")
    }
    
    // MARK: - State Management
    
    private func startStatePolling() {
        // Start playback for receiving
        do {
            try audioManager.startPlayback()
        } catch {
            print("❌ Failed to start playback: \(error)")
        }
        
        // Poll state every 100 ms. `.common` mode keeps it running while a list
        // scrolls (default mode pauses during tracking).
        let timer = Timer(timeInterval: 0.1, repeats: true) { [weak self] _ in
            self?.updateState()
        }
        RunLoop.main.add(timer, forMode: .common)
        stateTimer = timer
    }
    
    private static func currentRoomId() -> String? {
        guard let c = sassytalkie_relay_room_id() else { return nil }
        defer { sassytalkie_free_string(c) }
        let s = String(cString: c)
        return s.isEmpty ? nil : s
    }

    /// Runs on the main thread (timer). Reads state and applies it in the same
    /// turn: the old async hop could apply a pre-press snapshot after a press.
    private func updateState() {
        let state = sassytalkie_get_state()
        isPaired = sassytalkie_is_paired()
        if TrialStore.shouldWarn(entitled: isEntitled) {
            let left = TrialStore.sessionsRemaining()
            trialWarning = left == 1 ? "Last free session" : "\(left) free sessions left"
        } else {
            trialWarning = nil
        }

        // Rust ends a transmission by itself when another radio wins the floor
        // or the 60 s safety limit trips. The mic tap used to stay installed
        // (orange privacy dot, battery) with the button still "pressed".
        if isPTTPressed && state != 3 {
            isPTTPressed = false
            audioManager.stopRecording()
            _ = sassytalkie_ptt_release()
            if let c = sassytalkie_take_ptt_reject() {
                defer { sassytalkie_free_string(c) }
                pttRejectText = String(cString: c)
            }
        }

        let stateText: String
        switch state {
        case 0: // Idle
            isTransmitting = false
            isReceiving = false
            isConnected = false
            stateText = "Idle"
        case 1: // Connecting
            isTransmitting = false
            isReceiving = false
            isConnected = false
            stateText = "Connecting..."
        case 2: // Connected
            isTransmitting = false
            isReceiving = false
            isConnected = true
            stateText = "Listening"
        case 3: // Transmitting
            isTransmitting = true
            isReceiving = false
            isConnected = true
            stateText = "Transmitting"
        case 4: // Receiving
            isTransmitting = false
            isReceiving = true
            isConnected = true
            stateText = "Receiving"
        case 5: // Error
            isTransmitting = false
            isReceiving = false
            isConnected = false
            stateText = "Error"
        default:
            stateText = statusText
        }
        // Live TX/RX always shows; otherwise a recent message keeps the line.
        if state == 3 || state == 4 || Date() >= messageUntil {
            statusText = stateText
        }
        if isReceiving, let room = Self.currentRoomId() {
            TrialStore.noteQualifyingSession(room)
        }
    }
}
