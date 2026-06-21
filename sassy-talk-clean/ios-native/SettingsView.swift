//
//  SettingsView.swift
//  SassyTalkie
//
//  Copyright © 2025 Sassy Consulting LLC. All rights reserved.
//

import SwiftUI

struct SettingsView: View {
    @ObservedObject var viewModel: SassyTalkieViewModel
    @Environment(\.presentationMode) var presentationMode

    var body: some View {
        NavigationView {
            Form {
                Section(header: Text("ABOUT")) {
                    diagRow("Version", viewModel.version)

                    HStack {
                        Text("Status")
                        Spacer()
                        Text(viewModel.statusText)
                            .foregroundColor(viewModel.isConnected ? .stOnline : .stTextMuted)
                    }
                }

                Section(header: Text("CHANNEL")) {
                    HStack {
                        Text("Current Channel")
                        Spacer()
                        Text(String(format: "%02d", viewModel.channel))
                            .font(SassyTheme.mono(17, .semibold))
                            .foregroundColor(.stTeal)
                    }
                }

                Section(header: Text("AUDIO")) {
                    Text("Audio configuration is automatic")
                        .font(.caption)
                        .foregroundColor(.stTextMuted)
                }

                // On-the-go diagnostics — a read-only field dump for field
                // testing a shipped (release) build. No keys or peer identifiers,
                // safe to read aloud / screenshot in a support thread.
                Section(header: Text("DIAGNOSTICS")) {
                    diagRow("Device", UIDevice.current.model)
                    diagRow("iOS", UIDevice.current.systemVersion)
                    diagRow("Channel", String(format: "%02d", viewModel.channel))
                    diagRow("Connection", viewModel.statusText)
                    diagRow("Transmitting", viewModel.isTransmitting ? "yes" : "no")
                    diagRow("Receiving", viewModel.isReceiving ? "yes" : "no")
                }

                Section(header: Text("INFO")) {
                    Link("Privacy Policy", destination: URL(string: "https://sassyconsultingllc.github.io/sassy-talks/privacy-policy.html")!)
                    Link("Support", destination: URL(string: "https://sassyconsultingllc.github.io/sassy-talks/support.html")!)
                }
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button("Done") {
                        presentationMode.wrappedValue.dismiss()
                    }
                    .foregroundColor(.stTeal)
                }
            }
        }
        // Force dark so the grouped Form renders on slate-dark surfaces
        // (iOS-14-safe — .scrollContentBackground is iOS 16+). Teal accent
        // tints links + controls to match the Tauri reference.
        .preferredColorScheme(.dark)
        .accentColor(.stTeal)
    }

    /// Label / mono-value row used across the settings + diagnostics sections.
    private func diagRow(_ label: String, _ value: String) -> some View {
        HStack {
            Text(label)
            Spacer()
            Text(value)
                .font(SassyTheme.mono(14))
                .foregroundColor(.stTextSecondary)
        }
    }
}

struct SettingsView_Previews: PreviewProvider {
    static var previews: some View {
        SettingsView(viewModel: SassyTalkieViewModel())
    }
}
