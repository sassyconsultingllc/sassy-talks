// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-IOS3PAYWALLVW8
//
//  PaywallView.swift
//  SassyTalkie — StoreKit 2 paywall + relay promo (parity with Android
//  Entitlements.GateScreen). Promo field stays visible even when StoreKit fails.

import SwiftUI

struct PaywallView: View {
    @ObservedObject var viewModel: SassyTalkieViewModel
    @Environment(\.presentationMode) private var presentationMode

    @State private var busy = false
    @State private var promoBusy = false
    @State private var promoInput = ""
    @State private var status = ""

    var body: some View {
        NavigationView {
            ScrollView {
                VStack(spacing: 20) {
                    Image(systemName: "lock.fill")
                        .font(.system(size: 44))
                        .foregroundColor(.stCoral)

                    Text(headline)
                        .font(.title2).bold()
                        .foregroundColor(.stTextPrimary)
                        .multilineTextAlignment(.center)

                    Text(bodyCopy)
                        .font(.body)
                        .foregroundColor(.stTextSecondary)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal)

                    if !status.isEmpty {
                        Text(status)
                            .font(.caption)
                            .foregroundColor(.stWarning)
                            .multilineTextAlignment(.center)
                    }

                    Button(action: buy) {
                        HStack {
                            if busy { ProgressView().progressViewStyle(CircularProgressViewStyle(tint: .white)) }
                            Text("Unlock SassyTalkie")
                                .fontWeight(.semibold)
                        }
                        .frame(maxWidth: .infinity)
                        .padding()
                        .background(Color.stTeal)
                        .foregroundColor(.white)
                        .cornerRadius(SassyTheme.radiusMd)
                    }
                    .disabled(busy || promoBusy)

                    Button("Restore Purchase") { restore() }
                        .foregroundColor(.stTeal)
                        .disabled(busy || promoBusy)

                    // Promo always shown — a failed StoreKit catalog must not hide it
                    // (Android Play gate keeps Redeem promo reachable when Billing disconnects).
                    VStack(spacing: 10) {
                        Text("Have a promo code?")
                            .font(.caption)
                            .foregroundColor(.stTextMuted)
                        TextField("Promo code", text: $promoInput)
                            .autocapitalization(.allCharacters)
                            .disableAutocorrection(true)
                            .padding(12)
                            .background(Color.stBgMedium)
                            .cornerRadius(SassyTheme.radiusSm)
                            .foregroundColor(.stTextPrimary)
                            .disabled(promoBusy)
                        if promoBusy {
                            ProgressView().progressViewStyle(CircularProgressViewStyle(tint: .stTeal))
                        } else {
                            Button(action: redeemPromo) {
                                Text("Redeem promo")
                                    .fontWeight(.semibold)
                                    .frame(maxWidth: .infinity)
                                    .padding()
                                    .background(Color.stTeal.opacity(promoInput.trimmingCharacters(in: .whitespaces).isEmpty ? 0.4 : 1))
                                    .foregroundColor(.white)
                                    .cornerRadius(SassyTheme.radiusMd)
                            }
                            .disabled(promoInput.trimmingCharacters(in: .whitespaces).isEmpty)
                        }
                    }
                    .padding(.top, 8)

                    Spacer(minLength: 24)
                }
                .padding(24)
            }
            .background(Color.stBgDark.ignoresSafeArea())
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if TrialStore.mayUseRadio(entitled: viewModel.isEntitled) {
                    ToolbarItem(placement: .navigationBarLeading) {
                        Button("Back") { presentationMode.wrappedValue.dismiss() }
                            .foregroundColor(.stTeal)
                    }
                }
            }
        }
        .preferredColorScheme(.dark)
        .accentColor(.stTeal)
    }

    private var headline: String {
        if TrialStore.trialExhausted(entitled: viewModel.isEntitled) {
            return "You used your 5 free sessions"
        }
        return "This build arrived locked"
    }

    private var bodyCopy: String {
        if #available(iOS 15.0, *) {
            return "A one-time unlock keeps the radio working after the trial. Same product as Android (\(StoreKitEntitlements.productId)). Friends & family can redeem a promo below."
        }
        return "Purchasing needs iOS 15 or later. Promo codes and the 5-session trial still work on this device."
    }

    private func buy() {
        if #available(iOS 15.0, *) {
            busy = true
            status = ""
            Task {
                let result = await StoreKitEntitlements.purchase()
                await MainActor.run {
                    busy = false
                    switch result {
                    case .success(true):
                        viewModel.isEntitled = true
                        presentationMode.wrappedValue.dismiss()
                    case .success(false):
                        status = "Purchase cancelled or pending"
                    case .failure(let err):
                        status = err.localizedDescription
                    }
                }
            }
        } else {
            status = "Requires iOS 15+ for App Store purchase — use a promo code below"
        }
    }

    private func restore() {
        busy = true
        status = ""
        StoreKitEntitlements.refresh { ok in
            busy = false
            viewModel.isEntitled = ok
            status = ok ? "Restored" : "No purchase found"
            if ok { presentationMode.wrappedValue.dismiss() }
        }
    }

    private func redeemPromo() {
        let code = promoInput
        promoBusy = true
        status = ""
        DispatchQueue.global(qos: .userInitiated).async {
            let result = LicensePromo.redeemBlocking(code)
            DispatchQueue.main.async {
                promoBusy = false
                switch result {
                case .ok:
                    // Receipt-driven unlock — do not persist permanent StoreKit flag.
                    viewModel.isEntitled = StoreKitEntitlements.isUnlockedCached
                    presentationMode.wrappedValue.dismiss()
                case .invalidFormat:
                    status = "Enter a valid promo code (6–40 characters)"
                case .networkError:
                    status = "Can't reach the license server — check your connection"
                case .rejected(let message):
                    status = message
                }
            }
        }
    }
}
