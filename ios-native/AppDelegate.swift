// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-IOSAPPDELEG8N
//
//  AppDelegate.swift
//  Requests notification permission and registers for remote notifications so a
//  real APNs device token can be POSTed to /presence. Wake pushes (kind|type=
//  wake) post Notification.Name.sassyTalkieWake for a warm relay reconnect.
//
//  Push delivery still needs an Apple Developer APNs key on the relay worker
//  (APNS_* secrets) and a provisioning profile that includes Push Notifications.
//  remote-notification background mode is enabled so silent wakes can run the
//  warm-reconnect handler while the process is suspended.

import UIKit
import UserNotifications

final class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        requestPushPermissionAndRegister(application)
        if let remote = launchOptions?[.remoteNotification] as? [AnyHashable: Any] {
            handleWakePayload(remote, source: "launch")
        }
        return true
    }

    private func requestPushPermissionAndRegister(_ application: UIApplication) {
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) { granted, error in
            if let error {
                NSLog("SassyTalkie push: auth error %@", error.localizedDescription)
            }
            guard granted else {
                NSLog("SassyTalkie push: notification permission denied — cold wake unavailable")
                return
            }
            DispatchQueue.main.async {
                application.registerForRemoteNotifications()
            }
        }
    }

    func application(_ application: UIApplication,
                     didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        PresenceClient.rememberDeviceToken(deviceToken)
        if let room = currentRoomId() {
            PresenceClient.uploadCurrentToken(roomId: room)
        }
        NSLog("SassyTalkie push: APNs token registered (len=%d)", deviceToken.count)
    }

    func application(_ application: UIApplication,
                     didFailToRegisterForRemoteNotificationsWithError error: Error) {
        // Expected if provisioning lacks Push Notifications / APNs entitlement.
        NSLog("SassyTalkie push: APNs registration failed — %@", error.localizedDescription)
    }

    // Warm path when a remote notification arrives while the process is alive.
    func application(
        _ application: UIApplication,
        didReceiveRemoteNotification userInfo: [AnyHashable: Any],
        fetchCompletionHandler completionHandler: @escaping (UIBackgroundFetchResult) -> Void
    ) {
        let handled = handleWakePayload(userInfo, source: "remote")
        completionHandler(handled ? .newData : .noData)
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        let info = notification.request.content.userInfo
        if isWake(info) {
            _ = handleWakePayload(info, source: "foreground")
            // Still show so the operator knows a wake arrived.
            if #available(iOS 14.0, *) {
                completionHandler([.banner, .sound])
            } else {
                completionHandler([.alert, .sound])
            }
        } else {
            completionHandler([])
        }
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        _ = handleWakePayload(response.notification.request.content.userInfo, source: "tap")
        completionHandler()
    }

    @discardableResult
    private func handleWakePayload(_ userInfo: [AnyHashable: Any], source: String) -> Bool {
        guard isWake(userInfo) else { return false }
        let room = (userInfo["room"] as? String) ?? ""
        NSLog("SassyTalkie push: WAKE source=%@ room=%@", source, room)
        NotificationCenter.default.post(
            name: .sassyTalkieWake,
            object: nil,
            userInfo: room.isEmpty ? nil : ["room": room]
        )
        return true
    }

    private func isWake(_ userInfo: [AnyHashable: Any]) -> Bool {
        // Mirror Android: accept kind (historical) or type (older relay).
        let kind = (userInfo["kind"] as? String) ?? (userInfo["type"] as? String)
        if kind == "wake" { return true }
        // APNs often nests custom data under "aps" sibling keys — also check
        // a top-level data-style dictionary if the worker wraps the payload.
        if let data = userInfo["data"] as? [AnyHashable: Any] {
            let nested = (data["kind"] as? String) ?? (data["type"] as? String)
            return nested == "wake"
        }
        return false
    }

    private func currentRoomId() -> String? {
        guard let c = sassytalkie_relay_room_id() else { return nil }
        defer { sassytalkie_free_string(c) }
        let s = String(cString: c)
        return s.isEmpty ? nil : s
    }
}
