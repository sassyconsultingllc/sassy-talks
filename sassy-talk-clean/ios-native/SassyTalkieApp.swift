//
//  SassyTalkieApp.swift
//  SassyTalkie
//
//  Copyright © 2025 Sassy Consulting LLC. All rights reserved.
//

import SwiftUI

@main
struct SassyTalkieApp: App {
    
    init() {
        // Request microphone permission
        requestMicrophonePermission()
    }
    
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
    
    private func requestMicrophonePermission() {
        #if !targetEnvironment(simulator)
        AVAudioSession.sharedInstance().requestRecordPermission { granted in
            if granted {
                print("✅ Microphone permission granted")
            } else {
                print("❌ Microphone permission denied")
            }
        }
        #endif
    }
}
