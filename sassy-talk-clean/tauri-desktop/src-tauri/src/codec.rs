/// Codec Module - Opus Encoding/Decoding
/// 
/// Uses Opus codec for high-quality, low-latency voice compression
/// 48kHz sample rate, 20ms frames

use audiopus::{
    coder::{Encoder as OpusEncoderImpl, Decoder as OpusDecoderImpl},
    Application, Channels, SampleRate, Bitrate, Error as OpusError,
};
use thiserror::Error;

/// Codec error types
#[derive(Error, Debug)]
pub enum CodecError {
    #[error("Opus encoder error: {0}")]
    EncoderError(String),
    
    #[error("Opus decoder error: {0}")]
    DecoderError(String),
    
    #[error("Invalid frame size: {0}")]
    InvalidFrameSize(usize),
    
    #[error("Invalid sample rate: {0}")]
    InvalidSampleRate(u32),
}

/// Sample rate (Opus native)
pub const SAMPLE_RATE: u32 = 48000;

/// Frame duration in milliseconds
pub const FRAME_DURATION_MS: u32 = 20;

/// Frame size in samples (20ms at 48kHz)
pub const FRAME_SIZE: usize = 960;

/// Maximum packet size
const MAX_PACKET_SIZE: usize = 4000;

/// Opus encoder wrapper
pub struct OpusEncoder {
    encoder: OpusEncoderImpl,
    frame_size: usize,
    channels: Channels,
}

impl OpusEncoder {
    /// Create new Opus encoder
    pub fn new() -> Result<Self, CodecError> {
        let sample_rate = SampleRate::Hz48000;
        let channels = Channels::Mono;
        let application = Application::Voip; // Optimized for voice
        
        let mut encoder = OpusEncoderImpl::new(sample_rate, channels, application)
            .map_err(|e| CodecError::EncoderError(format!("{:?}", e)))?;
        
        // Configure for low latency voice
        encoder.set_bitrate(Bitrate::BitsPerSecond(32000))
            .map_err(|e| CodecError::EncoderError(format!("{:?}", e)))?;
        
        encoder.set_vbr(true)
            .map_err(|e| CodecError::EncoderError(format!("{:?}", e)))?;
        
        encoder.set_complexity(10) // Max quality
            .map_err(|e| CodecError::EncoderError(format!("{:?}", e)))?;
        
        Ok(Self {
            encoder,
            frame_size: FRAME_SIZE,
            channels,
        })
    }
    
    /// Encode PCM samples to Opus
    /// 
    /// Input: &[i16] - PCM samples (960 samples for 20ms at 48kHz)
    /// Output: Vec<u8> - Compressed Opus packet (typically 40-80 bytes)
    pub fn encode(&mut self, pcm: &[i16]) -> Result<Vec<u8>, CodecError> {
        if pcm.len() != self.frame_size {
            return Err(CodecError::InvalidFrameSize(pcm.len()));
        }
        
        let mut output = vec![0u8; MAX_PACKET_SIZE];
        
        let encoded_size = self.encoder
            .encode(pcm, &mut output)
            .map_err(|e| CodecError::EncoderError(format!("{:?}", e)))?;
        
        output.truncate(encoded_size);
        Ok(output)
    }
    
    /// Get frame size
    pub fn frame_size(&self) -> usize {
        self.frame_size
    }
}

/// Opus decoder wrapper
pub struct OpusDecoder {
    decoder: OpusDecoderImpl,
    frame_size: usize,
    channels: Channels,
}

impl OpusDecoder {
    /// Create new Opus decoder
    pub fn new() -> Result<Self, CodecError> {
        let sample_rate = SampleRate::Hz48000;
        let channels = Channels::Mono;
        
        let decoder = OpusDecoderImpl::new(sample_rate, channels)
            .map_err(|e| CodecError::DecoderError(format!("{:?}", e)))?;
        
        Ok(Self {
            decoder,
            frame_size: FRAME_SIZE,
            channels,
        })
    }
    
    /// Decode Opus packet to PCM samples
    /// 
    /// Input: &[u8] - Compressed Opus packet (40-80 bytes typically)
    /// Output: Vec<i16> - PCM samples (960 samples)
    pub fn decode(&mut self, opus_data: &[u8]) -> Result<Vec<i16>, CodecError> {
        let mut output = vec![0i16; self.frame_size];
        
        let decoded_size = self.decoder
            .decode(Some(opus_data), &mut output, false)
            .map_err(|e| CodecError::DecoderError(format!("{:?}", e)))?;
        
        if decoded_size != self.frame_size {
            output.truncate(decoded_size);
        }
        
        Ok(output)
    }
    
    /// Decode packet loss concealment (PLC)
    /// 
    /// Generates audio samples when packet is lost
    pub fn decode_plc(&mut self) -> Result<Vec<i16>, CodecError> {
        let mut output = vec![0i16; self.frame_size];
        
        let decoded_size = self.decoder
            .decode(None, &mut output, false)
            .map_err(|e| CodecError::DecoderError(format!("{:?}", e)))?;
        
        if decoded_size != self.frame_size {
            output.truncate(decoded_size);
        }
        
        Ok(output)
    }
    
    /// Get frame size
    pub fn frame_size(&self) -> usize {
        self.frame_size
    }
}

/// Audio frame for transmission
#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub samples: Vec<i16>,
    pub timestamp: u64,
}

impl AudioFrame {
    /// Create new audio frame
    pub fn new(size: usize) -> Self {
        Self {
            samples: vec![0; size],
            timestamp: 0,
        }
    }
    
    /// Create from samples
    pub fn from_samples(samples: Vec<i16>, timestamp: u64) -> Self {
        Self { samples, timestamp }
    }
    
    /// Get duration in milliseconds
    pub fn duration_ms(&self) -> u32 {
        (self.samples.len() as u32 * 1000) / SAMPLE_RATE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_creation() {
        let encoder = OpusEncoder::new();
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_decoder_creation() {
        let decoder = OpusDecoder::new();
        assert!(decoder.is_ok());
    }

    #[test]
    fn test_encode_decode() {
        let mut encoder = OpusEncoder::new().unwrap();
        let mut decoder = OpusDecoder::new().unwrap();
        
        // Create test signal (440Hz sine wave)
        let mut samples = vec![0i16; FRAME_SIZE];
        for (i, sample) in samples.iter_mut().enumerate() {
            let t = i as f32 / SAMPLE_RATE as f32;
            *sample = (f32::sin(2.0 * std::f32::consts::PI * 440.0 * t) * 16000.0) as i16;
        }
        
        // Encode
        let encoded = encoder.encode(&samples).unwrap();
        println!("Encoded {} samples to {} bytes", samples.len(), encoded.len());
        assert!(encoded.len() < samples.len() * 2); // Should be compressed
        
        // Decode
        let decoded = decoder.decode(&encoded).unwrap();
        println!("Decoded {} bytes to {} samples", encoded.len(), decoded.len());
        assert_eq!(decoded.len(), samples.len());
    }

    #[test]
    fn test_packet_loss_concealment() {
        let mut decoder = OpusDecoder::new().unwrap();
        
        let plc_samples = decoder.decode_plc().unwrap();
        assert_eq!(plc_samples.len(), FRAME_SIZE);
    }

    #[test]
    fn test_invalid_frame_size() {
        let mut encoder = OpusEncoder::new().unwrap();
        
        let wrong_size = vec![0i16; 100]; // Wrong size
        let result = encoder.encode(&wrong_size);
        assert!(result.is_err());
    }
}
