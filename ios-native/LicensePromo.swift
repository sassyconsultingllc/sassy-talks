// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-IOSPROMORED9M
//
//  LicensePromo.swift
//  Promo-code redemption against the relay `/license/promo` endpoint
//  (parity with Android LicensePromo.kt). Receipt is time-limited; do not
//  set the permanent StoreKit unlocked flag.

import Foundation
import UIKit

enum LicensePromo {
    static let licenseApiBase = "https://relay.sassyconsultingllc.com"
    private static let refreshWhenRemainingSec: TimeInterval = 15 * 24 * 3600
    private static let licenseRe = try! NSRegularExpression(
        pattern: "^SASSY(?:-[A-HJ-NP-Z2-9]{5}){4}$")
    private static let promoRe = try! NSRegularExpression(
        pattern: "^[A-Z0-9-]{6,40}$")

    private static let keyLicense = "promo_license_code"
    private static let keyKind = "promo_credential_kind"
    private static let keyReceiptExp = "promo_receipt_exp"
    private static let keyInstallId = "promo_install_id"

    enum RedeemResult {
        case ok
        case invalidFormat
        case networkError
        case rejected(String)
    }

    /// True when the cached credential kind is promo (may be expired).
    static var isPromoCredential: Bool {
        UserDefaults.standard.string(forKey: keyKind) == "promo"
    }

    static func hasValidReceipt() -> Bool {
        guard isPromoCredential else { return false }
        let exp = UserDefaults.standard.double(forKey: keyReceiptExp)
        return exp > Date().timeIntervalSince1970
    }

    /// Mirrors worker normalizePromo: 6–40 chars A-Z 0-9 -, not a license key.
    static func normalizeCode(_ raw: String) -> String? {
        let code = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            .uppercased()
            .replacingOccurrences(of: "\\s+", with: "", options: .regularExpression)
        let range = NSRange(code.startIndex..., in: code)
        if licenseRe.firstMatch(in: code, range: range) != nil { return nil }
        guard promoRe.firstMatch(in: code, range: range) != nil else { return nil }
        return code
    }

    static func redeemBlocking(_ rawCode: String) -> RedeemResult {
        guard let code = normalizeCode(rawCode) else { return .invalidFormat }
        switch callPromo(code) {
        case .ok(let expiresAt):
            persist(code: code, expiresAt: expiresAt)
            return .ok
        case .rejected(let message):
            return .rejected(message)
        case .networkError:
            return .networkError
        }
    }

    static func refreshIfNeeded(completion: @escaping (Bool) -> Void) {
        let defaults = UserDefaults.standard
        guard let code = defaults.string(forKey: keyLicense),
              defaults.string(forKey: keyKind) == "promo" else {
            completion(false)
            return
        }
        let exp = defaults.double(forKey: keyReceiptExp)
        let now = Date().timeIntervalSince1970
        if exp <= now {
            completion(false)
            return
        }
        if exp - now > refreshWhenRemainingSec {
            completion(true)
            return
        }
        DispatchQueue.global(qos: .utility).async {
            switch callPromo(code) {
            case .ok(let expiresAt):
                defaults.set(expiresAt, forKey: keyReceiptExp)
                completion(true)
            case .rejected:
                defaults.removeObject(forKey: keyReceiptExp)
                defaults.removeObject(forKey: keyKind)
                defaults.removeObject(forKey: keyLicense)
                completion(false)
            case .networkError:
                completion(exp > now)
            }
        }
    }

    private static func persist(code: String, expiresAt: Double) {
        // Receipt-driven only — never set StoreKitEntitlements.persistUnlocked.
        let defaults = UserDefaults.standard
        defaults.set(code, forKey: keyLicense)
        defaults.set("promo", forKey: keyKind)
        defaults.set(expiresAt, forKey: keyReceiptExp)
    }

    private enum ApiResult {
        case ok(Double)
        case rejected(String)
        case networkError
    }

    private static func callPromo(_ code: String) -> ApiResult {
        guard let url = URL(string: "\(licenseApiBase)/license/promo") else {
            return .networkError
        }
        let body: [String: String] = [
            "code": code,
            "device_id": deviceId(),
            "device_name": UIDevice.current.model,
            "app_version": Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "",
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: body) else {
            return .networkError
        }
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = data
        req.timeoutInterval = 10

        let sem = DispatchSemaphore(value: 0)
        var result: ApiResult = .networkError
        PinnedURLSession.shared.dataTask(with: req) { data, response, error in
            defer { sem.signal() }
            if error != nil {
                result = .networkError
                return
            }
            let code = (response as? HTTPURLResponse)?.statusCode ?? 0
            let json = (data.flatMap {
                try? JSONSerialization.jsonObject(with: $0) as? [String: Any]
            }) ?? [:]
            if (200..<300).contains(code), json["ok"] as? Bool == true {
                let exp = (json["expires_at"] as? Double)
                    ?? Double(json["expires_at"] as? Int ?? 0)
                result = .ok(exp)
            } else if code >= 500 {
                result = .networkError
            } else {
                let msg = (json["error"] as? String).flatMap { $0.isEmpty ? nil : $0 }
                    ?? "Promo rejected"
                result = .rejected(msg)
            }
        }.resume()
        _ = sem.wait(timeout: .now() + 12)
        return result
    }

    private static func deviceId() -> String {
        if let id = UIDevice.current.identifierForVendor?.uuidString, !id.isEmpty {
            return id
        }
        if let existing = UserDefaults.standard.string(forKey: keyInstallId), !existing.isEmpty {
            return existing
        }
        let fresh = UUID().uuidString
        UserDefaults.standard.set(fresh, forKey: keyInstallId)
        return fresh
    }
}
