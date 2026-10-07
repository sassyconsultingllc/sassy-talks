// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-IOS2STOREKIT24
//
//  StoreKitEntitlements.swift
//  SassyTalkie — StoreKit 2 equivalent of Android Play Billing `sassytalkie_unlock`,
//  plus relay promo receipts (LicensePromo) matching Play Entitlements.kt.

import Foundation
import StoreKit

/// Non-consumable product id — must match App Store Connect and Android Play.
enum StoreKitEntitlements {
    static let productId = "sassytalkie_unlock"
    private static let unlockedKey = "storekit_unlocked"

    static var isUnlockedCached: Bool {
        #if DEBUG
        return true
        #else
        if UserDefaults.standard.bool(forKey: unlockedKey) { return true }
        return LicensePromo.hasValidReceipt()
        #endif
    }

    static func persistUnlocked(_ value: Bool) {
        UserDefaults.standard.set(value, forKey: unlockedKey)
    }

    private static var updatesTask: Task<Void, Never>?

    /// Listen to `Transaction.updates` for the life of the app. Start once at
    /// launch, before any purchase. Without it, an Ask-to-Buy purchase that a
    /// parent approves later, an offer-code redemption, a purchase interrupted
    /// mid-flow, or a refund never reached the app: `purchase()` returned
    /// `.pending` and the unlock never happened, and unfinished transactions
    /// are redelivered forever (an App Review rejection reason).
    /// `onChange` runs on the main actor with the current entitlement.
    static func startObservingTransactions(onChange: @escaping (Bool) -> Void) {
        guard updatesTask == nil else { return }
        guard #available(iOS 15.0, *) else { return }
        updatesTask = Task.detached(priority: .background) {
            for await result in Transaction.updates {
                guard case .verified(let tx) = result, tx.productID == productId else { continue }
                let owned = tx.revocationDate == nil
                persistUnlocked(owned)
                await tx.finish()
                let entitled = owned || LicensePromo.hasValidReceipt()
                await MainActor.run { onChange(entitled) }
            }
        }
    }

    /// Silent reconcile. A live promo receipt refreshes against the relay;
    /// anything else (no promo, or a promo that lapsed) asks the App Store, so a
    /// promo user who later bought the app is not locked out when the promo
    /// expires. Always completes on the main thread (callers mutate UI state).
    static func refresh(completion: @escaping (Bool) -> Void) {
        let finish: (Bool) -> Void = { ok in DispatchQueue.main.async { completion(ok) } }
        #if DEBUG
        finish(true)
        return
        #endif
        if LicensePromo.hasValidReceipt() {
            LicensePromo.refreshIfNeeded { ok in
                if ok || LicensePromo.hasValidReceipt() {
                    finish(true)
                } else {
                    checkAppStore(finish)
                }
            }
            return
        }
        checkAppStore(finish)
    }

    /// The "Restore Purchase" button: force an App Store sync first (Apple's
    /// restore on StoreKit 2; it may ask the user to sign in), then reconcile.
    /// `currentEntitlements` alone can miss a purchase on a new device until
    /// the store syncs. Completes on the main thread.
    static func restore(completion: @escaping (Bool) -> Void) {
        if #available(iOS 15.0, *) {
            Task {
                try? await AppStore.sync()
                refresh(completion: completion)
            }
        } else {
            refresh(completion: completion)
        }
    }

    private static func checkAppStore(_ finish: @escaping (Bool) -> Void) {
        if #available(iOS 15.0, *) {
            Task { finish(await refreshStoreKit2()) }
        } else {
            finish(UserDefaults.standard.bool(forKey: unlockedKey) || LicensePromo.hasValidReceipt())
        }
    }

    @available(iOS 15.0, *)
    private static func refreshStoreKit2() async -> Bool {
        #if DEBUG
        return true
        #else
        do {
            for await result in Transaction.currentEntitlements {
                if case .verified(let tx) = result, tx.productID == productId,
                   tx.revocationDate == nil {
                    persistUnlocked(true)
                    return true
                }
            }
            // Keep a still-valid promo receipt even if StoreKit has no purchase.
            if LicensePromo.hasValidReceipt() { return true }
            persistUnlocked(false)
            return false
        }
        #endif
    }

    @available(iOS 15.0, *)
    static func purchase() async -> Result<Bool, Error> {
        do {
            let products = try await Product.products(for: [productId])
            guard let product = products.first else {
                return .failure(StoreKitError.productMissing)
            }
            let result = try await product.purchase()
            switch result {
            case .success(let verification):
                let tx = try checkVerified(verification)
                await tx.finish()
                persistUnlocked(true)
                return .success(true)
            case .userCancelled:
                return .success(false)
            case .pending:
                return .success(false)
            @unknown default:
                return .success(false)
            }
        } catch {
            return .failure(error)
        }
    }

    @available(iOS 15.0, *)
    private static func checkVerified<T>(_ result: VerificationResult<T>) throws -> T {
        switch result {
        case .unverified(_, let error):
            throw error
        case .verified(let safe):
            return safe
        }
    }

    enum StoreKitError: LocalizedError {
        case productMissing
        var errorDescription: String? {
            "Product \"\(productId)\" is not on the App Store. Confirm it is published, or tap Restore."
        }
    }
}
