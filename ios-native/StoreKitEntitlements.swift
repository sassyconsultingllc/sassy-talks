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

    /// Silent reconcile: promo receipts refresh against the relay; otherwise
    /// App Store (reinstall / refund).
    static func refresh(completion: @escaping (Bool) -> Void) {
        #if DEBUG
        completion(true)
        return
        #endif
        if LicensePromo.isPromoCredential {
            if !LicensePromo.hasValidReceipt() {
                completion(false)
                return
            }
            LicensePromo.refreshIfNeeded { ok in
                completion(ok || LicensePromo.hasValidReceipt())
            }
            return
        }
        if #available(iOS 15.0, *) {
            Task {
                let ok = await refreshStoreKit2()
                await MainActor.run { completion(ok) }
            }
        } else {
            completion(isUnlockedCached)
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
