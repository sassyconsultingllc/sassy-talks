// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-4EIRK7Q6WK7W
//
//  AudioManager.swift
//  SassyTalkie
//
//  Copyright © 2025 Sassy Consulting LLC. All rights reserved.
//

import Foundation
import AVFoundation

/// Audio manager using AVAudioEngine
/// Bridges iOS audio to Rust core
class AudioManager: NSObject {
    
    // MARK: - Properties
    
    private let audioEngine = AVAudioEngine()
    private let inputNode: AVAudioInputNode
    private let outputNode: AVAudioOutputNode
    private var sourceNode: AVAudioSourceNode?

    private var isRecording = false
    private var isPlaying = false

    // Audio format: 48kHz, mono, 16-bit PCM (Rust core)
    private let sampleRate: Double = 48000
    private let channelCount: UInt32 = 1
    private let frameSize: UInt32 = 960 // 20ms at 48kHz

    /// Render-thread scratch for Rust's int16 output, allocated once. The render
    /// block used to build a fresh Swift array every callback — a heap
    /// allocation on the real-time thread, a classic source of glitches.
    private static let renderScratchCapacity = 4096
    private let renderScratch = UnsafeMutablePointer<Int16>.allocate(capacity: AudioManager.renderScratchCapacity)
    
    // MARK: - Initialization
    
    override init() {
        self.inputNode = audioEngine.inputNode
        self.outputNode = audioEngine.outputNode
        super.init()
        
        setupAudioSession()
        observeInterruptions()
    }

    private func observeInterruptions() {
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(handleInterruption),
            name: AVAudioSession.interruptionNotification,
            object: AVAudioSession.sharedInstance()
        )
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(handleRouteChange),
            name: AVAudioSession.routeChangeNotification,
            object: AVAudioSession.sharedInstance()
        )
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(handleEngineConfigurationChange),
            name: .AVAudioEngineConfigurationChange,
            object: audioEngine
        )
    }

    /// AirPods / headset / CarPlay connecting changes the hardware format, and
    /// AVAudioEngine STOPS itself and posts this. Nothing restarted it, so
    /// after pairing earbuds the app went silent until relaunch. Restart the
    /// engine and, if transmitting, reinstall the mic tap for the new format.
    @objc private func handleEngineConfigurationChange(_ note: Notification) {
        DispatchQueue.main.async {
            let wasRecording = self.isRecording
            if wasRecording {
                self.inputNode.removeTap(onBus: 0)
                self.isRecording = false
            }
            do {
                if wasRecording {
                    try self.startRecording()
                } else if self.isPlaying && !self.audioEngine.isRunning {
                    try self.audioEngine.start()
                }
            } catch {
                print("❌ Audio engine restart after route change failed: \(error)")
            }
        }
    }

    @objc private func handleInterruption(_ note: Notification) {
        guard let info = note.userInfo,
              let typeVal = info[AVAudioSessionInterruptionTypeKey] as? UInt,
              let type = AVAudioSession.InterruptionType(rawValue: typeVal) else { return }
        if type == .ended {
            try? AVAudioSession.sharedInstance().setActive(true)
            if isPlaying || isRecording {
                try? audioEngine.start()
            }
        }
    }

    @objc private func handleRouteChange(_ note: Notification) {
        // Bluetooth SCO / speaker flips change the hardware sample rate; the
        // input tap already converts whatever the node delivers. Re-activate
        // so playAndRecord stays live after a headset unplug.
        try? AVAudioSession.sharedInstance().setActive(true)
    }
    
    // MARK: - Audio Session
    
    private func setupAudioSession() {
        let session = AVAudioSession.sharedInstance()
        do {
            try session.setCategory(.playAndRecord, mode: .voiceChat, options: [.defaultToSpeaker, .allowBluetooth])
            // One Opus frame per IO cycle keeps mic-to-wire latency near 20 ms
            // (the default can be ~100 ms of buffered input on some devices).
            try? session.setPreferredSampleRate(sampleRate)
            try? session.setPreferredIOBufferDuration(0.02)
            try session.setActive(true)
            print("✅ Audio session configured")
        } catch {
            print("❌ Failed to setup audio session: \(error)")
        }
    }
    
    // MARK: - Recording
    
    func startRecording() throws {
        guard !isRecording else { return }

        // Format the Rust core expects: 48 kHz mono int16.
        let target = AVAudioFormat(
            commonFormat: .pcmFormatInt16,
            sampleRate: sampleRate,
            channels: channelCount,
            interleaved: false
        )!

        // Install the tap with the input node's ACTUAL hardware format, never a
        // hardcoded 48 kHz one. With .allowBluetooth the input route can be an
        // 8/16 kHz SCO headset (and the node is usually float32); passing a
        // mismatched format makes installTap raise an Obj-C NSException that
        // Swift's do/catch cannot catch — a hard crash on PTT. Convert whatever
        // the hardware delivers to `target` before handing it to Rust.
        let hwFormat = inputNode.inputFormat(forBus: 0)
        let converter = AVAudioConverter(from: hwFormat, to: target)

        inputNode.installTap(onBus: 0, bufferSize: frameSize, format: hwFormat) { [weak self] buffer, _ in
            guard let self = self else { return }
            guard let converter = converter else {
                // Converter unavailable — best effort with the raw buffer.
                self.processInputBuffer(buffer)
                return
            }
            let ratio = target.sampleRate / hwFormat.sampleRate
            let capacity = AVAudioFrameCount((Double(buffer.frameLength) * ratio).rounded(.up)) + 1
            guard let out = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: capacity) else { return }
            var fed = false
            var convError: NSError?
            let status = converter.convert(to: out, error: &convError) { _, inputStatus in
                if fed {
                    inputStatus.pointee = .noDataNow
                    return nil
                }
                fed = true
                inputStatus.pointee = .haveData
                return buffer
            }
            if status != .error, convError == nil, out.frameLength > 0 {
                self.processInputBuffer(out)
            }
        }

        try audioEngine.start()
        isRecording = true
        print("🎤 Recording started")
    }
    
    func stopRecording() {
        guard isRecording else { return }
        
        inputNode.removeTap(onBus: 0)
        isRecording = false
        
        if !isPlaying {
            audioEngine.stop()
        }
        print("🎤 Recording stopped")
    }
    
    private func processInputBuffer(_ buffer: AVAudioPCMBuffer) {
        guard let channelData = buffer.int16ChannelData else { return }
        
        let frameLength = Int(buffer.frameLength)
        let samples = Array(UnsafeBufferPointer(start: channelData[0], count: frameLength))
        
        // Send to Rust. baseAddress is non-nil whenever samples is non-empty;
        // the FFI param is _Nonnull, so guard rather than force-unwrap.
        samples.withUnsafeBufferPointer { pointer in
            if let base = pointer.baseAddress, samples.count > 0 {
                _ = sassytalkie_process_audio_input(base, samples.count)
            }
        }
    }
    
    // MARK: - Playback
    
    func startPlayback() throws {
        guard !isPlaying else { return }
        
        let graphFormat = AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: 1)!

        let node = AVAudioSourceNode(format: graphFormat) { [weak self] _, _, frameCount, audioBufferList -> OSStatus in
            self?.fillOutputBuffer(audioBufferList, frameCount: frameCount) ?? noErr
        }

        audioEngine.attach(node)
        audioEngine.connect(node, to: audioEngine.mainMixerNode, format: graphFormat)
        sourceNode = node

        if !audioEngine.isRunning {
            try audioEngine.start()
        }

        isPlaying = true
        print("🔊 Playback started")
    }
    
    func stopPlayback() {
        guard isPlaying else { return }
        
        isPlaying = false
        
        if !isRecording {
            audioEngine.stop()
        }
        print("🔊 Playback stopped")
    }
    
    /// Real-time render callback: no allocation, no blocking (Rust's playout
    /// read is try-lock only). Mono source, so every channel buffer gets the
    /// same samples.
    private func fillOutputBuffer(_ bufferList: UnsafeMutablePointer<AudioBufferList>, frameCount: UInt32) -> OSStatus {
        let ablPointer = UnsafeMutableAudioBufferListPointer(bufferList)
        let total = Int(frameCount)
        var offset = 0
        while offset < total {
            let chunk = min(total - offset, Self.renderScratchCapacity)
            let written = Int(sassytalkie_get_audio_output(renderScratch, chunk))
            for buffer in ablPointer {
                guard let ptr = buffer.mData else { continue }
                let floats = ptr.assumingMemoryBound(to: Float.self)
                let capacity = Int(buffer.mDataByteSize) / MemoryLayout<Float>.size
                let end = min(offset + chunk, capacity)
                guard offset < end else { continue }
                for i in offset..<end {
                    let j = i - offset
                    floats[i] = j < written ? Float(renderScratch[j]) / 32768.0 : 0
                }
            }
            offset += chunk
        }
        return noErr
    }
    
    // MARK: - Cleanup
    
    deinit {
        NotificationCenter.default.removeObserver(self)
        stopRecording()
        stopPlayback()
        audioEngine.stop()
        renderScratch.deallocate()
    }
}
