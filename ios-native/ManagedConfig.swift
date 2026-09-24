// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-IOS9MDMCONFIG2
//
//  ManagedConfig.swift
//  Reads MDM app-config (`com.apple.configuration.managed`) for the optional
//  enrollment token and force_session_wipe. Room id is not authorization —
//  the token is.

import Foundation

enum ManagedConfig {
    private static let managedKey = "com.apple.configuration.managed"
    static let enrollmentTokenKey = "enrollment_token"
    static let forceSessionWipeKey = "force_session_wipe"

    private static var managedDict: [String: Any]? {
        UserDefaults.standard.dictionary(forKey: managedKey)
    }

    static func apply() {
        let dict = managedDict
        let token = dict?[enrollmentTokenKey] as? String
        if let token = token, !token.isEmpty {
            _ = token.withCString { sassytalkie_set_enrollment_token($0) }
        } else {
            _ = sassytalkie_set_enrollment_token(nil)
        }
    }

    /// MDM `force_session_wipe` — parity with Android ManagedConfig / SessionWipe.
    static var forceSessionWipe: Bool {
        if let b = managedDict?[forceSessionWipeKey] as? Bool { return b }
        if let n = managedDict?[forceSessionWipeKey] as? NSNumber { return n.boolValue }
        return false
    }
}
