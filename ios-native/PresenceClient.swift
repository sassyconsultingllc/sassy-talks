// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-IOSAPNSPRES7K
//
//  PresenceClient.swift
//  Thin client for the relay's /presence endpoint (parity with Android
//  PresenceClient.kt). Registers this install's APNs device token under
//  (room, peer) so the worker can fire a wake push when the WS has dropped.
//
//  Auth: POST/DELETE require Authorization: Bearer <room capability> minted
//  from /auth?room=&peer= (peer-bound — see presence.js verifyCapabilityIdentity).
//  Never POST an empty or invented token; wait for a real APNs device token.

import Foundation
import UIKit

enum PresenceClient {
    static let relayBase = "https://relay.sassyconsultingllc.com"
    private static let tokenDefaultsKey = "apns_device_token_hex"
    private static let peerDefaultsKey = "install_peer_id"

    /// Stable per-install peer id — must match RelayClient's WS `peer=` query.
    static var peerId: String {
        if let existing = UserDefaults.standard.string(forKey: peerDefaultsKey), !existing.isEmpty {
            return existing
        }
        let fresh = UIDevice.current.identifierForVendor?.uuidString ?? UUID().uuidString
        UserDefaults.standard.set(fresh, forKey: peerDefaultsKey)
        return fresh
    }

    /// Hex-encoded APNs device token, or nil until registration succeeds.
    static var storedDeviceToken: String? {
        get {
            let s = UserDefaults.standard.string(forKey: tokenDefaultsKey)
            return (s?.isEmpty == false) ? s : nil
        }
        set {
            if let newValue, !newValue.isEmpty {
                UserDefaults.standard.set(newValue, forKey: tokenDefaultsKey)
            } else {
                UserDefaults.standard.removeObject(forKey: tokenDefaultsKey)
            }
        }
    }

    static func rememberDeviceToken(_ data: Data) {
        let hex = data.map { String(format: "%02x", $0) }.joined()
        guard !hex.isEmpty else { return }
        storedDeviceToken = hex
    }

    /// Fire-and-forget: upload the stored APNs token for [roomId] when one exists.
    static func uploadCurrentToken(roomId: String) {
        guard !roomId.isEmpty, let token = storedDeviceToken, !token.isEmpty else { return }
        DispatchQueue.global(qos: .utility).async {
            _ = upload(roomId: roomId, deviceToken: token)
        }
    }

    /// Returns true on 2xx. Never throws; never logs the token value.
    @discardableResult
    static func upload(roomId: String, deviceToken: String) -> Bool {
        guard !roomId.isEmpty, !deviceToken.isEmpty else { return false }
        let peer = peerId
        guard let cap = fetchCapabilityToken(room: roomId, peer: peer) else { return false }
        let body: [String: String] = [
            "room": roomId,
            "peer": peer,
            "token": deviceToken,
            "platform": "apns",
        ]
        guard let url = URL(string: "\(relayBase)/presence"),
              let data = try? JSONSerialization.data(withJSONObject: body) else { return false }
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("Bearer \(cap)", forHTTPHeaderField: "Authorization")
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = data
        return syncBool(req)
    }

    /// DELETE the (room, peer) row — call on session wipe.
    @discardableResult
    static func remove(roomId: String) -> Bool {
        guard !roomId.isEmpty else { return false }
        let peer = peerId
        guard let cap = fetchCapabilityToken(room: roomId, peer: peer) else { return false }
        let body: [String: String] = ["room": roomId, "peer": peer]
        guard let url = URL(string: "\(relayBase)/presence"),
              let data = try? JSONSerialization.data(withJSONObject: body) else { return false }
        var req = URLRequest(url: url)
        req.httpMethod = "DELETE"
        req.setValue("Bearer \(cap)", forHTTPHeaderField: "Authorization")
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = data
        return syncBool(req)
    }

    private static func fetchCapabilityToken(room: String, peer: String) -> String? {
        var comps = URLComponents(string: "\(relayBase)/auth")
        comps?.queryItems = [
            URLQueryItem(name: "room", value: room),
            URLQueryItem(name: "peer", value: peer),
        ]
        guard let url = comps?.url else { return nil }
        let sem = DispatchSemaphore(value: 0)
        var token: String?
        PinnedURLSession.shared.dataTask(with: url) { data, response, _ in
            defer { sem.signal() }
            let code = (response as? HTTPURLResponse)?.statusCode ?? 0
            guard (200..<300).contains(code),
                  let data,
                  let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let t = json["token"] as? String, !t.isEmpty else {
                NSLog("SassyTalkie presence: auth failed HTTP %d", code)
                return
            }
            token = t
        }.resume()
        _ = sem.wait(timeout: .now() + 15)
        return token
    }

    private static func syncBool(_ req: URLRequest) -> Bool {
        let sem = DispatchSemaphore(value: 0)
        var ok = false
        PinnedURLSession.shared.dataTask(with: req) { _, response, _ in
            defer { sem.signal() }
            let code = (response as? HTTPURLResponse)?.statusCode ?? 0
            ok = (200..<300).contains(code)
            if !ok {
                NSLog("SassyTalkie presence: HTTP %d", code)
            }
        }.resume()
        _ = sem.wait(timeout: .now() + 15)
        return ok
    }
}

extension Notification.Name {
    /// Posted when a wake push (kind/type=wake) arrives or is tapped.
    static let sassyTalkieWake = Notification.Name("sassyTalkieWake")
}
