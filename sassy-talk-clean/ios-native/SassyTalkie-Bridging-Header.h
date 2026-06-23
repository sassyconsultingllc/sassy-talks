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

// ── Bluetooth (CoreBluetooth bridge — Swift discovers, Rust tracks state) ──

/// BLE service UUID string (must free with sassytalkie_free_string)
const char* _Nonnull sassytalkie_bt_service_uuid(void);

/// Register a peer discovered by the Swift central
bool sassytalkie_bt_device_found(const char* _Nullable id, const char* _Nullable name, int32_t rssi);

/// Remove a peer reported lost / disconnected
bool sassytalkie_bt_device_lost(const char* _Nullable id);

/// Number of currently-discovered peers
size_t sassytalkie_bt_peer_count(void);

/// JSON array of discovered peers (must free with sassytalkie_free_string)
const char* _Nonnull sassytalkie_bt_get_peers_json(void);

/// Mark a peer as the actively-connected device
bool sassytalkie_bt_set_connected(const char* _Nullable id);

/// Clear the active BLE connection
void sassytalkie_bt_clear_connected(void);

// ── Crypto / key agreement (shared core — parity with Android) ──

/// Install a pre-shared key (QR session key) from base64 (32 bytes decoded).
/// After this, TX is AES-256-GCM encrypted and RX is decrypted + replay-checked.
bool sassytalkie_set_psk(const char* _Nullable key_b64);

/// Begin a classical X25519 key exchange. Returns our base64 public key (must
/// free with sassytalkie_free_string). Complete with the function below.
char* _Nullable sassytalkie_key_exchange_init(void);

/// Complete the classical key exchange with the peer's base64 public key,
/// installing the AEAD session. Returns true on success.
bool sassytalkie_key_exchange_complete(const char* _Nullable remote_b64);

/// This build's capability bitmap (hybrid-PQC support) — same value Android
/// advertises in its heartbeat caps byte.
uint8_t sassytalkie_local_capabilities(void);

/// Begin a hybrid PQC handshake. Returns base64 initiator message (must free with
/// sassytalkie_free_string), or NULL if no PSK is installed / not initialized.
char* _Nullable sassytalkie_hybrid_handshake_init(void);

/// Responder: given the peer's base64 initiator message, install the session and
/// return the base64 responder message (must free with sassytalkie_free_string),
/// or NULL on failure.
char* _Nullable sassytalkie_hybrid_handshake_respond(const char* _Nullable init_b64);

/// Initiator: complete with the peer's base64 responder message, installing the
/// post-quantum session. Returns true on success.
bool sassytalkie_hybrid_handshake_complete(const char* _Nullable resp_b64);

#endif /* SassyTalkie_Bridging_Header_h */
