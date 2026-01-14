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
                    HStack {
                        Text("Version")
                        Spacer()
                        Text(viewModel.version)
                            .foregroundColor(.gray)
                    }
                    
                    HStack {
                        Text("Status")
                        Spacer()
                        Text(viewModel.statusText)
                            .foregroundColor(viewModel.isConnected ? .green : .gray)
                    }
                }
                
                Section(header: Text("CHANNEL")) {
                    HStack {
                        Text("Current Channel")
                        Spacer()
                        Text(String(format: "%02d", viewModel.channel))
                            .font(.system(.body, design: .monospaced))
                            .foregroundColor(.cyan)
                    }
                }
                
                Section(header: Text("AUDIO")) {
                    Text("Audio configuration is automatic")
                        .font(.caption)
                        .foregroundColor(.gray)
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
                }
            }
        }
    }
}

struct SettingsView_Previews: PreviewProvider {
    static var previews: some View {
        SettingsView(viewModel: SassyTalkieViewModel())
    }
}
