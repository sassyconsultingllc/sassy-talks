// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-IOS6APNSWAKE4R
//
//  PushWakePolicy.swift
//  APNs equivalent of Android FcmWakePolicy — policy + registration hooks.
//
//  What the app does in code (PresenceClient + AppDelegate):
//    • Request notification permission and registerForRemoteNotifications.
//    • When a real APNs device token arrives, POST /presence with
//      Authorization: Bearer <peer-bound capability>, platform=apns
//      (never invent/empty token).
//    • On kind|type=wake, warm-reconnect the relay; cold path is a user-visible
//      notification the operator taps (mic cannot start from a silent push).
//
//  Worker path (cloudflare-worker apns.js / wake-push.js): presence rows with
//  platform=apns are woken via Apple HTTP/2, not FCM. Requires owner-set
//  APNS_* secrets on the worker (see cloudflare-worker/.env.example).
//
//  Still requires an Apple Developer account (owner work, not inventable here):
//    • APNs Auth Key (.p8) configured on the relay worker.
//    • Push Notifications capability on the App ID / provisioning profile
//      (aps-environment in SassyTalkie.entitlements; SystemCapabilities in
//      the Xcode project).
//    • Info.plist `remote-notification` background mode (enabled now that
//      the worker can send APNs wakes).
//
//  iOS cannot start microphone capture from a silent push (same class of
//  restriction as Android API 34+ FGS+mic from FCM).

import Foundation

enum PushWakePolicy {
    enum Path { case warmReconnect, visibleNotification }

    static func path(appIsActive: Bool) -> Path {
        appIsActive ? .warmReconnect : .visibleNotification
    }

    /// iOS never allows starting AVAudioEngine recording from a silent push.
    static func mayStartMicrophoneFromSilentPush() -> Bool { false }

    static func mayStartUIFromSilentPush() -> Bool { false }

    static let wakeCategoryId = "sassytalkie_apns_wake"
}
