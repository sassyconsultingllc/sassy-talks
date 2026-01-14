//
//  ContentView.swift
//  SassyTalkie
//
//  Copyright © 2025 Sassy Consulting LLC. All rights reserved.
//

import SwiftUI

struct ContentView: View {
    @StateObject private var viewModel = SassyTalkieViewModel()
    
    var body: some View {
        NavigationView {
            VStack(spacing: 0) {
                // Header
                headerView
                
                Spacer()
                
                // Status
                statusView
                
                Spacer()
                
                // Channel selector
                channelSelector
                
                Spacer()
                
                // PTT Button
                pttButton
                
                Spacer()
            }
            .padding()
            .background(Color(hex: "1A1A2E"))
            .navigationTitle("SassyTalkie")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button(action: { viewModel.showingSettings.toggle() }) {
                        Image(systemName: "gear")
                            .foregroundColor(.cyan)
                    }
                }
            }
            .sheet(isPresented: $viewModel.showingSettings) {
                SettingsView(viewModel: viewModel)
            }
        }
    }
    
    // MARK: - Header
    
    private var headerView: some View {
        VStack(spacing: 8) {
            Text("SASSYTALKIE")
                .font(.system(size: 32, weight: .bold, design: .monospaced))
                .foregroundColor(.orange)
            
            Text("v\(viewModel.version)")
                .font(.caption)
                .foregroundColor(.gray)
        }
        .padding(.top, 20)
    }
    
    // MARK: - Status
    
    private var statusView: some View {
        VStack(spacing: 12) {
            // Connection status
            HStack {
                Circle()
                    .fill(viewModel.statusColor)
                    .frame(width: 12, height: 12)
                
                Text(viewModel.statusText)
                    .font(.headline)
                    .foregroundColor(.white)
            }
            
            // State indicator
            if viewModel.isTransmitting {
                Text("TRANSMITTING")
                    .font(.system(size: 24, weight: .bold))
                    .foregroundColor(.orange)
                    .padding(.horizontal, 20)
                    .padding(.vertical, 10)
                    .background(
                        RoundedRectangle(cornerRadius: 8)
                            .fill(Color.orange.opacity(0.2))
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: 8)
                            .stroke(Color.orange, lineWidth: 2)
                    )
            } else if viewModel.isReceiving {
                Text("RECEIVING")
                    .font(.system(size: 24, weight: .bold))
                    .foregroundColor(.cyan)
                    .padding(.horizontal, 20)
                    .padding(.vertical, 10)
                    .background(
                        RoundedRectangle(cornerRadius: 8)
                            .fill(Color.cyan.opacity(0.2))
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: 8)
                            .stroke(Color.cyan, lineWidth: 2)
                    )
            }
        }
    }
    
    // MARK: - Channel Selector
    
    private var channelSelector: some View {
        VStack(spacing: 12) {
            Text("CHANNEL")
                .font(.caption)
                .foregroundColor(.gray)
            
            HStack(spacing: 20) {
                Button(action: { viewModel.decrementChannel() }) {
                    Image(systemName: "minus.circle.fill")
                        .font(.system(size: 32))
                        .foregroundColor(.cyan)
                }
                
                Text(String(format: "%02d", viewModel.channel))
                    .font(.system(size: 48, weight: .bold, design: .monospaced))
                    .foregroundColor(.white)
                    .frame(width: 100)
                
                Button(action: { viewModel.incrementChannel() }) {
                    Image(systemName: "plus.circle.fill")
                        .font(.system(size: 32))
                        .foregroundColor(.cyan)
                }
            }
        }
    }
    
    // MARK: - PTT Button
    
    private var pttButton: some View {
        Button(action: {}) {
            ZStack {
                Circle()
                    .fill(viewModel.isPTTPressed ? Color.orange : Color(hex: "252546"))
                    .frame(width: 200, height: 200)
                    .overlay(
                        Circle()
                            .stroke(viewModel.isPTTPressed ? Color.orange : Color.cyan, lineWidth: 4)
                    )
                    .shadow(color: viewModel.isPTTPressed ? .orange.opacity(0.6) : .clear, radius: 20)
                
                VStack(spacing: 8) {
                    Image(systemName: "mic.fill")
                        .font(.system(size: 48))
                        .foregroundColor(viewModel.isPTTPressed ? .white : .cyan)
                    
                    Text("PUSH TO TALK")
                        .font(.system(size: 14, weight: .bold))
                        .foregroundColor(viewModel.isPTTPressed ? .white : .cyan)
                }
            }
        }
        .buttonStyle(PTTButtonStyle(viewModel: viewModel))
        .padding(.bottom, 40)
    }
}

// MARK: - PTT Button Style

struct PTTButtonStyle: ButtonStyle {
    @ObservedObject var viewModel: SassyTalkieViewModel
    
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.95 : 1.0)
            .animation(.spring(response: 0.2), value: configuration.isPressed)
            .onChange(of: configuration.isPressed) { pressed in
                if pressed {
                    viewModel.pttPress()
                } else {
                    viewModel.pttRelease()
                }
            }
    }
}

// MARK: - Color Extension

extension Color {
    init(hex: String) {
        let scanner = Scanner(string: hex)
        var rgb: UInt64 = 0
        scanner.scanHexInt64(&rgb)
        
        let r = Double((rgb >> 16) & 0xFF) / 255.0
        let g = Double((rgb >> 8) & 0xFF) / 255.0
        let b = Double(rgb & 0xFF) / 255.0
        
        self.init(red: r, green: g, blue: b)
    }
}

// MARK: - Preview

struct ContentView_Previews: PreviewProvider {
    static var previews: some View {
        ContentView()
    }
}
