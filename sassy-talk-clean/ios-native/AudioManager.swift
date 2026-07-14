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
    
    private var isRecording = false
    private var isPlaying = false
    
    // Audio format: 48kHz, mono, 16-bit PCM
    private let sampleRate: Double = 48000
    private let channelCount: UInt32 = 1
    private let frameSize: UInt32 = 960 // 20ms at 48kHz
    
    // MARK: - Initialization
    
    override init() {
        self.inputNode = audioEngine.inputNode
        self.outputNode = audioEngine.outputNode
        super.init()
        
        setupAudioSession()
    }
    
    // MARK: - Audio Session
    
    private func setupAudioSession() {
        let session = AVAudioSession.sharedInstance()
        do {
            try session.setCategory(.playAndRecord, mode: .voiceChat, options: [.defaultToSpeaker, .allowBluetooth])
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
        
        let format = AVAudioFormat(
            commonFormat: .pcmFormatInt16,
            sampleRate: sampleRate,
            channels: channelCount,
            interleaved: false
        )!
        
        let sourceNode = AVAudioSourceNode { [weak self] _, _, frameCount, audioBufferList -> OSStatus in
            self?.fillOutputBuffer(audioBufferList, frameCount: frameCount) ?? noErr
        }
        
        audioEngine.attach(sourceNode)
        audioEngine.connect(sourceNode, to: outputNode, format: format)
        
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
    
    private func fillOutputBuffer(_ bufferList: UnsafeMutablePointer<AudioBufferList>, frameCount: UInt32) -> OSStatus {
        let ablPointer = UnsafeMutableAudioBufferListPointer(bufferList)
        
        for buffer in ablPointer {
            let samples = buffer.mData?.assumingMemoryBound(to: Int16.self)
            
            if let samples = samples {
                let count = Int(frameCount)
                
                // Get audio from Rust
                let written = sassytalkie_get_audio_output(samples, count)
                
                // Fill remaining with silence
                if written < count {
                    for i in written..<count {
                        samples[i] = 0
                    }
                }
            }
        }
        
        return noErr
    }
    
    // MARK: - Cleanup
    
    deinit {
        stopRecording()
        stopPlayback()
        audioEngine.stop()
    }
}
