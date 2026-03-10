/// Audio Pipeline - TX/RX threads that wire the full audio path
///
/// TX path (on PTT press):
///   Mic → AudioEngine::read_audio → VoiceEncoder::encode → pack_wire_frame → Transport::send (encrypted)
///
/// RX path (always running when connected):
///   Transport::receive (decrypted) → unpack_wire_frame → VoiceDecoder::decode → AudioCache → AudioTrack
///
/// Also handles the TranscriptionBridge callback to Kotlin for speech-to-text.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use log::{error, info, warn};

use crate::audio::AudioEngine;
use crate::codec::{VoiceEncoder, VoiceDecoder, CODEC_FRAME_SIZE, COMPRESSED_FRAME_SIZE};
use crate::transport::TransportManager;
use crate::audio_cache::AudioCache;
use crate::users::UserRegistry;

/// Maximum sender ID length on the wire
const MAX_SENDER_ID_LEN: usize = 32;

/// Maximum device name length on the wire
const MAX_DEVICE_NAME_LEN: usize = 64;

/// Encapsulate an audio frame for transport over the wire.
///
/// Format: [channel:1][sender_id_len:1][sender_id:N][name_len:1][device_name:M][timestamp:8][compressed_audio:484]
pub fn pack_wire_frame(channel: u8, sender_id: &str, device_name: &str, timestamp: u64, compressed: &[u8]) -> Vec<u8> {
    let id_bytes = sender_id.as_bytes();
    let id_len = id_bytes.len().min(MAX_SENDER_ID_LEN);
    let name_bytes = device_name.as_bytes();
    let name_len = name_bytes.len().min(MAX_DEVICE_NAME_LEN);
    let mut packet = Vec::with_capacity(1 + 1 + id_len + 1 + name_len + 8 + compressed.len());
    packet.push(channel);
    packet.push(id_len as u8);
    packet.extend_from_slice(&id_bytes[..id_len]);
    packet.push(name_len as u8);
    packet.extend_from_slice(&name_bytes[..name_len]);
    packet.extend_from_slice(&timestamp.to_le_bytes());
    packet.extend_from_slice(compressed);
    packet
}

/// Parse a wire frame back into its components.
///
/// Returns (channel, sender_id, device_name, timestamp, compressed_audio) or error.
pub fn unpack_wire_frame(data: &[u8]) -> Result<(u8, String, String, u64, Vec<u8>), String> {
    // Minimum: channel(1) + id_len(1) + name_len(1) + timestamp(8) = 11
    if data.len() < 11 {
        return Err(format!("Wire frame too short: {} bytes", data.len()));
    }

    let channel = data[0];
    let id_len = data[1] as usize;

    if id_len > MAX_SENDER_ID_LEN || data.len() < 2 + id_len + 1 {
        return Err(format!("Invalid sender_id length: {}", id_len));
    }

    let sender_id = String::from_utf8_lossy(&data[2..2 + id_len]).to_string();

    let name_len_offset = 2 + id_len;
    let name_len = data[name_len_offset] as usize;

    if name_len > MAX_DEVICE_NAME_LEN || data.len() < name_len_offset + 1 + name_len + 8 {
        return Err(format!("Invalid device_name length: {}", name_len));
    }

    let name_start = name_len_offset + 1;
    let device_name = String::from_utf8_lossy(&data[name_start..name_start + name_len]).to_string();

    let ts_offset = name_start + name_len;
    let timestamp = u64::from_le_bytes([
        data[ts_offset], data[ts_offset + 1], data[ts_offset + 2], data[ts_offset + 3],
        data[ts_offset + 4], data[ts_offset + 5], data[ts_offset + 6], data[ts_offset + 7],
    ]);

    let audio_offset = ts_offset + 8;
    let compressed = data[audio_offset..].to_vec();

    Ok((channel, sender_id, device_name, timestamp, compressed))
}

/// Get current time in milliseconds since epoch
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Spawn the TX thread: captures mic audio, encodes, encrypts, and sends while PTT is held.
///
/// The thread runs in a loop while `tx_running` is true. It only captures+sends when
/// `ptt_pressed` is true.
pub fn spawn_tx_thread(
    tx_running: Arc<AtomicBool>,
    ptt_pressed: Arc<AtomicBool>,
    current_channel: Arc<AtomicU8>,
    audio: Arc<Mutex<AudioEngine>>,
    transport: Arc<Mutex<TransportManager>>,
    local_sender_id: String,
    local_device_name: String,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("sassy-tx".into())
        .spawn(move || {
            info!("TX thread started");

            let mut encoder = VoiceEncoder::new();
            let mut pcm_buffer = vec![0i16; CODEC_FRAME_SIZE];
            let mut was_transmitting = false;

            while tx_running.load(Ordering::Relaxed) {
                if !ptt_pressed.load(Ordering::Relaxed) {
                    // Not transmitting
                    if was_transmitting {
                        // PTT released: stop recording
                        let eng = audio.lock().unwrap();
                        let _ = eng.stop_recording();
                        was_transmitting = false;
                        encoder.reset();
                        info!("TX: stopped recording (PTT released)");
                    }
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }

                // PTT is pressed: transmit
                if !was_transmitting {
                    // PTT just pressed: start recording
                    let eng = audio.lock().unwrap();
                    match eng.start_recording() {
                        Ok(()) => {
                            was_transmitting = true;
                            info!("TX: started recording (PTT pressed)");
                        }
                        Err(e) => {
                            error!("TX: failed to start recording: {}", e);
                            thread::sleep(Duration::from_millis(50));
                            continue;
                        }
                    }
                }

                // Read one frame from mic
                let samples_read = {
                    let eng = audio.lock().unwrap();
                    match eng.read_audio(&mut pcm_buffer) {
                        Ok(n) => n,
                        Err(e) => {
                            warn!("TX: read_audio failed: {}", e);
                            0
                        }
                    }
                };

                if samples_read < CODEC_FRAME_SIZE {
                    // Incomplete frame, wait for more data
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }

                // Encode with ADPCM
                let compressed = encoder.encode(&pcm_buffer[..CODEC_FRAME_SIZE]);

                // Pack wire frame (includes device name for receiver display)
                let channel = current_channel.load(Ordering::Relaxed);
                let timestamp = now_ms();
                let wire_data = pack_wire_frame(channel, &local_sender_id, &local_device_name, timestamp, &compressed);

                // Send through transport (encrypted inside TransportManager::send)
                let mut tm = transport.lock().unwrap();
                if let Err(e) = tm.send(&wire_data) {
                    warn!("TX: send failed: {}", e);
                }
            }

            // Cleanup
            if was_transmitting {
                let eng = audio.lock().unwrap();
                let _ = eng.stop_recording();
            }
            info!("TX thread stopped");
        })
        .expect("Failed to spawn TX thread")
}

/// Spawn the RX thread: receives, decrypts, decodes, feeds into AudioCache, and plays back.
///
/// Also calls the TranscriptionBridge callback if available.
pub fn spawn_rx_thread(
    rx_running: Arc<AtomicBool>,
    current_channel: Arc<AtomicU8>,
    audio: Arc<Mutex<AudioEngine>>,
    transport: Arc<Mutex<TransportManager>>,
    audio_cache: Arc<Mutex<AudioCache>>,
    user_registry: Arc<Mutex<UserRegistry>>,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("sassy-rx".into())
        .spawn(move || {
            info!("RX thread started");

            let mut decoder = VoiceDecoder::new();
            let mut recv_buffer = vec![0u8; 2048]; // generous buffer for encrypted + wire header
            let mut playback_started = false;

            while rx_running.load(Ordering::Relaxed) {
                // Receive from transport (decrypted inside TransportManager::receive)
                let bytes_received = {
                    let mut tm = transport.lock().unwrap();
                    match tm.receive(&mut recv_buffer) {
                        Ok(n) => n,
                        Err(e) => {
                            if !e.contains("would block") && !e.contains("No active transport") {
                                warn!("RX: receive failed: {}", e);
                            }
                            0
                        }
                    }
                };

                if bytes_received == 0 {
                    // No data available. Tick the audio cache to check for completed utterances
                    // and drain playback queue.
                    let mut cache = audio_cache.lock().unwrap();
                    cache.tick();

                    // If we're in Queue mode, try to play the next frame from cache
                    if let Some((_sender, samples)) = cache.next_playback_frame() {
                        drop(cache);
                        if !playback_started {
                            let eng = audio.lock().unwrap();
                            let _ = eng.start_playing();
                            playback_started = true;
                        }
                        let eng = audio.lock().unwrap();
                        let _ = eng.write_audio(&samples);
                    } else {
                        drop(cache);
                    }

                    thread::sleep(Duration::from_millis(5));
                    continue;
                }

                // Unpack wire frame (now includes device_name)
                let (channel, sender_id, device_name, timestamp, compressed) = match unpack_wire_frame(&recv_buffer[..bytes_received]) {
                    Ok(parsed) => parsed,
                    Err(e) => {
                        warn!("RX: invalid wire frame: {}", e);
                        continue;
                    }
                };

                // Filter by channel
                let my_channel = current_channel.load(Ordering::Relaxed);
                if channel != my_channel {
                    continue;
                }

                // Validate compressed size
                if compressed.len() != COMPRESSED_FRAME_SIZE {
                    warn!("RX: unexpected compressed size: {} (expected {})", compressed.len(), COMPRESSED_FRAME_SIZE);
                    continue;
                }

                // Auto-register the sender in UserRegistry so we have their name
                // and sync to AudioCache for mute/favorite filtering
                {
                    let mut reg = user_registry.lock().unwrap();
                    reg.register_user(&sender_id, &device_name);
                }
                {
                    let reg = user_registry.lock().unwrap();
                    let is_fav = reg.is_favorite(&sender_id);
                    let is_muted = reg.is_muted(&sender_id);
                    let mut cache = audio_cache.lock().unwrap();
                    cache.update_user_info(&sender_id, &device_name, is_fav, is_muted);
                }

                // Decode ADPCM
                let pcm_samples = decoder.decode(&compressed);

                // Feed into audio cache
                let mut cache = audio_cache.lock().unwrap();
                let passthrough = cache.ingest_frame(&sender_id, timestamp, pcm_samples.clone());
                cache.tick();

                // Play audio: either passthrough (Live mode) or from queue
                let samples_to_play = if let Some(direct) = passthrough {
                    Some(direct)
                } else {
                    cache.next_playback_frame().map(|(_, s)| s)
                };

                drop(cache);

                if let Some(samples) = samples_to_play {
                    if !playback_started {
                        let eng = audio.lock().unwrap();
                        let _ = eng.start_playing();
                        playback_started = true;
                    }
                    let eng = audio.lock().unwrap();
                    let _ = eng.write_audio(&samples);
                }

                // Invoke TranscriptionBridge callback to Kotlin with the actual device name
                let (is_favorite, is_muted) = {
                    let reg = user_registry.lock().unwrap();
                    (reg.is_favorite(&sender_id), reg.is_muted(&sender_id))
                };

                call_transcription_bridge(&sender_id, &device_name, &pcm_samples, is_favorite, is_muted);
            }

            // Cleanup
            if playback_started {
                let eng = audio.lock().unwrap();
                let _ = eng.stop_playing();
            }
            info!("RX thread stopped");
        })
        .expect("Failed to spawn RX thread")
}

/// Call the Kotlin TranscriptionBridge.onAudioReceived callback via JNI.
///
/// Uses the cached GlobalRef from nativeInit (resolved on the main thread with
/// the app classloader) so that native RX threads can find the class.
/// This avoids ClassNotFoundException on attached native threads which only
/// have the system classloader.
fn call_transcription_bridge(
    sender_id: &str,
    sender_name: &str,
    pcm_samples: &[i16],
    is_favorite: bool,
    is_muted: bool,
) {
    use jni::objects::{JValue, JClass, GlobalRef};
    use jni::sys::{JNI_TRUE, JNI_FALSE};

    // Use the cached class ref (resolved on the main thread during nativeInit)
<<<<<<< HEAD
    let bridge_ref: &GlobalRef = match crate::jni_bridge::get_transcription_bridge_class() {
=======
    let bridge_ref = match crate::jni_bridge::get_transcription_bridge_class() {
>>>>>>> a2f85e424db1ffaac3647b50ff7dc3fc9d934ea5
        Some(r) => r,
        None => return, // TranscriptionBridge not available (class not found at init)
    };

    let vm = match crate::jni_bridge::get_jvm() {
        Ok(v) => v,
        Err(_) => return, // JVM not available (running tests)
    };

    let mut env = match vm.attach_current_thread() {
        Ok(e) => e,
        Err(_) => return,
    };

    // Create JNI arguments
    let j_sender_id = match env.new_string(sender_id) {
        Ok(s) => s,
        Err(_) => return,
    };
    let j_sender_name = match env.new_string(sender_name) {
        Ok(s) => s,
        Err(_) => return,
    };

    // Create short array for PCM samples
    let j_pcm = match env.new_short_array(pcm_samples.len() as i32) {
        Ok(a) => a,
        Err(_) => return,
    };
    if env.set_short_array_region(&j_pcm, 0, pcm_samples).is_err() {
        return;
    }

    let j_fav = if is_favorite { JNI_TRUE } else { JNI_FALSE };
    let j_muted = if is_muted { JNI_TRUE } else { JNI_FALSE };

<<<<<<< HEAD
    // Call static method using cached GlobalRef (carries app classloader context)
    // Safety: GlobalRef -> JObject -> JClass cast is valid for class references
    let bridge_class = unsafe { JClass::from_raw(bridge_ref.as_obj().as_raw()) };
    let result = env.call_static_method(
        &bridge_class,
=======
    // Call static method: TranscriptionBridge.onAudioReceived(...)
    // Using the cached GlobalRef which carries the app classloader context
    let result = env.call_static_method(
        <&jni::objects::JClass>::from(bridge_ref.as_obj()),
>>>>>>> a2f85e424db1ffaac3647b50ff7dc3fc9d934ea5
        "onAudioReceived",
        "(Ljava/lang/String;Ljava/lang/String;[SZZ)V",
        &[
            JValue::Object(&j_sender_id.into()),
            JValue::Object(&j_sender_name.into()),
            JValue::Object(&j_pcm.into()),
            JValue::Bool(j_fav),
            JValue::Bool(j_muted),
        ],
    );

    // Clear any pending exception so it doesn't crash the RX thread
    if result.is_err() {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
<<<<<<< HEAD
    // Don't drop bridge_class - it's borrowed from the global ref, not owned
    std::mem::forget(bridge_class);
=======
>>>>>>> a2f85e424db1ffaac3647b50ff7dc3fc9d934ea5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wire_frame_roundtrip() {
        let channel = 5u8;
        let sender_id = "abc123def456";
        let device_name = "John's Galaxy S24";
        let timestamp = 1700000000000u64;
        let compressed = vec![42u8; COMPRESSED_FRAME_SIZE];

        let packed = pack_wire_frame(channel, sender_id, device_name, timestamp, &compressed);
        let (ch, sid, name, ts, audio) = unpack_wire_frame(&packed).unwrap();

        assert_eq!(ch, channel);
        assert_eq!(sid, sender_id);
        assert_eq!(name, device_name);
        assert_eq!(ts, timestamp);
        assert_eq!(audio, compressed);
    }

    #[test]
    fn test_wire_frame_empty_fields() {
        let packed = pack_wire_frame(1, "", "", 100, &[1, 2, 3]);
        let (ch, sid, name, ts, audio) = unpack_wire_frame(&packed).unwrap();
        assert_eq!(ch, 1);
        assert_eq!(sid, "");
        assert_eq!(name, "");
        assert_eq!(ts, 100);
        assert_eq!(audio, vec![1, 2, 3]);
    }

    #[test]
    fn test_wire_frame_too_short() {
        let result = unpack_wire_frame(&[0; 5]);
        assert!(result.is_err());
    }

    #[test]
    fn test_wire_frame_invalid_sender_len() {
        let mut data = vec![0u8; 20];
        data[1] = 200; // sender_id_len > MAX_SENDER_ID_LEN
        let result = unpack_wire_frame(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_wire_frame_invalid_name_len() {
        // Valid sender_id_len = 0, then name_len = 200 (too large)
        let mut data = vec![0u8; 20];
        data[1] = 0; // sender_id_len = 0
        data[2] = 200; // name_len > MAX_DEVICE_NAME_LEN
        let result = unpack_wire_frame(&data);
        assert!(result.is_err());
    }
}
