//
//  SassyTalkie-Bridging-Header.h
//  SassyTalkie iOS
//
//  Copyright © 2025 Sassy Consulting LLC. All rights reserved.
//

#ifndef SassyTalkie_Bridging_Header_h
#define SassyTalkie_Bridging_Header_h

#import <Foundation/Foundation.h>

// Rust library FFI functions

/// Initialize the SassyTalkie library
bool sassytalkie_init(void);

/// Shutdown the library
void sassytalkie_shutdown(void);

/// Get version string (must free with sassytalkie_free_string)
const char* _Nonnull sassytalkie_get_version(void);

/// Free string allocated by library
void sassytalkie_free_string(char* _Nullable s);

/// Set current channel (1-99)
bool sassytalkie_set_channel(uint8_t channel);

/// Get current channel
uint8_t sassytalkie_get_channel(void);

/// Press PTT button (start transmission)
bool sassytalkie_ptt_press(void);

/// Release PTT button (stop transmission)
bool sassytalkie_ptt_release(void);

/// Connect to peer device
bool sassytalkie_connect(uint32_t device_id);

/// Disconnect from peer
bool sassytalkie_disconnect(void);

/// Start listening for incoming audio
bool sassytalkie_start_listening(void);

/// Get current state
/// 0=Idle, 1=Connecting, 2=Connected, 3=Transmitting, 4=Receiving, 5=Error
uint8_t sassytalkie_get_state(void);

/// Process audio input from AVAudioEngine
/// audio_data: PCM samples (16-bit signed)
/// sample_count: Number of samples
bool sassytalkie_process_audio_input(const int16_t* _Nonnull audio_data, size_t sample_count);

/// Get audio output for AVAudioEngine
/// buffer: Output buffer for PCM samples
/// buffer_size: Maximum samples to write
/// Returns: Number of samples written
size_t sassytalkie_get_audio_output(int16_t* _Nonnull buffer, size_t buffer_size);

#endif /* SassyTalkie_Bridging_Header_h */
