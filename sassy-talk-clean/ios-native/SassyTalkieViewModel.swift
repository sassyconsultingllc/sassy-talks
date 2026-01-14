//
//  SassyTalkieViewModel.swift
//  SassyTalkie
//
//  Copyright © 2025 Sassy Consulting LLC. All rights reserved.
//

import Foundation
import SwiftUI
import Combine

/// View model for SassyTalkie app
class SassyTalkieViewModel: ObservableObject {
    
    // MARK: - Published Properties
    
    @Published var channel: UInt8 = 1
    @Published var isPTTPressed: Bool = false
    @Published var isTransmitting: Bool = false
    @Published var isReceiving: Bool = false
    @Published var isConnected: Bool = false
    @Published var statusText: String = "Initializing..."
    @Published var showingSettings: Bool = false
    
    var version: String {
        let cString = sassytalkie_get_version()
        let version = String(cString: cString)
        sassytalkie_free_string(UnsafeMutablePointer(mutating: cString))
        return version
    }
    
    var statusColor: Color {
        if isTransmitting {
            return .orange
        } else if isReceiving {
            return .cyan
        } else if isConnected {
            return .green
        } else {
            return .gray
        }
    }
    
    // MARK: - Private Properties
    
    private let audioManager = AudioManager()
    private var stateTimer: Timer?
    
    // MARK: - Initialization
    
    init() {
        // Initialize Rust library
        let success = sassytalkie_init()
        if success {
            print("✅ SassyTalkie initialized")
            statusText = "Ready"
            
            // Start listening
            _ = sassytalkie_start_listening()
            isConnected = true
            statusText = "Listening"
            
            // Start state polling
            startStatePolling()
        } else {
            print("❌ Failed to initialize SassyTalkie")
            statusText = "Error"
        }
    }
    
    deinit {
        stateTimer?.invalidate()
        sassytalkie_shutdown()
    }
    
    // MARK: - Channel Control
    
    func incrementChannel() {
        if channel < 99 {
            channel += 1
            _ = sassytalkie_set_channel(channel)
        }
    }
    
    func decrementChannel() {
        if channel > 1 {
            channel -= 1
            _ = sassytalkie_set_channel(channel)
        }
    }
    
    // MARK: - PTT Control
    
    func pttPress() {
        guard !isPTTPressed else { return }
        
        isPTTPressed = true
        
        let success = sassytalkie_ptt_press()
        if success {
            do {
                try audioManager.startRecording()
                print("🎤 PTT pressed")
            } catch {
                print("❌ Failed to start recording: \(error)")
                isPTTPressed = false
                _ = sassytalkie_ptt_release()
            }
        } else {
            print("❌ Failed to start PTT")
            isPTTPressed = false
        }
    }
    
    func pttRelease() {
        guard isPTTPressed else { return }
        
        isPTTPressed = false
        audioManager.stopRecording()
        _ = sassytalkie_ptt_release()
        print("🎤 PTT released")
    }
    
    // MARK: - State Management
    
    private func startStatePolling() {
        // Start playback for receiving
        do {
            try audioManager.startPlayback()
        } catch {
            print("❌ Failed to start playback: \(error)")
        }
        
        // Poll state every 100ms
        stateTimer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            self?.updateState()
        }
    }
    
    private func updateState() {
        let state = sassytalkie_get_state()
        
        DispatchQueue.main.async {
            switch state {
            case 0: // Idle
                self.isTransmitting = false
                self.isReceiving = false
                self.isConnected = false
                self.statusText = "Idle"
                
            case 1: // Connecting
                self.isTransmitting = false
                self.isReceiving = false
                self.isConnected = false
                self.statusText = "Connecting..."
                
            case 2: // Connected
                self.isTransmitting = false
                self.isReceiving = false
                self.isConnected = true
                self.statusText = "Listening"
                
            case 3: // Transmitting
                self.isTransmitting = true
                self.isReceiving = false
                self.isConnected = true
                self.statusText = "Transmitting"
                
            case 4: // Receiving
                self.isTransmitting = false
                self.isReceiving = true
                self.isConnected = true
                self.statusText = "Receiving"
                
            case 5: // Error
                self.isTransmitting = false
                self.isReceiving = false
                self.isConnected = false
                self.statusText = "Error"
                
            default:
                break
            }
        }
    }
}
