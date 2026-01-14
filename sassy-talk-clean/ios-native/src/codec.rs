/// Codec Module - Opus Encoding/Decoding for iOS
/// 
/// Same as desktop version - Opus compression for voice

use audiopus::{
    coder::{Encoder as OpusEncoderImpl, Decoder as OpusDecoderImpl},
    Application, Channels, SampleRate, Bitrate,
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CodecError {
    #[error("Opus encoder error: {0}")]
    EncoderError(String),
    
    #[error("Opus decoder error: {0}")]
    DecoderError(String),
    
    #[error("Invalid frame size: {0}")]
    InvalidFrameSize(usize),
}

/// Sample rate (48kHz)
pub const SAMPLE_RATE: u32 = 48000;

/// Frame duration (20ms)
pub const FRAME_DURATION_MS: u32 = 20;

/// Frame size (960 samples)
pub const FRAME_SIZE: usize = 960;

const MAX_PACKET_SIZE: usize = 4000;

/// Opus encoder
pub struct OpusEncoder {
    encoder: OpusEncoderImpl,
    frame_size: usize,
}

impl OpusEncoder {
    /// Create new encoder
    pub fn new() -> Result<Self, CodecError> {
        let sample_rate = SampleRate::Hz48000;
        let channels = Channels::Mono;
        let application = Application::Voip;
        
        let mut encoder = OpusEncoderImpl::new(sample_rate, channels, application)
            .map_err(|e| CodecError::EncoderError(format!("{:?}", e)))?;
        
        encoder.set_bitrate(Bitrate::BitsPerSecond(32000))
            .map_err(|e| CodecError::EncoderError(format!("{:?}", e)))?;
        
        encoder.set_vbr(true)
            .map_err(|e| CodecError::EncoderError(format!("{:?}", e)))?;
        
        encoder.set_complexity(10)
            .map_err(|e| CodecError::EncoderError(format!("{:?}", e)))?;
        
        Ok(Self {
            encoder,
            frame_size: FRAME_SIZE,
        })
    }
    
    /// Encode PCM to Opus
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

/// Opus decoder
pub struct OpusDecoder {
    decoder: OpusDecoderImpl,
    frame_size: usize,
}

impl OpusDecoder {
    /// Create new decoder
    pub fn new() -> Result<Self, CodecError> {
        let sample_rate = SampleRate::Hz48000;
        let channels = Channels::Mono;
        
        let decoder = OpusDecoderImpl::new(sample_rate, channels)
            .map_err(|e| CodecError::DecoderError(format!("{:?}", e)))?;
        
        Ok(Self {
            decoder,
            frame_size: FRAME_SIZE,
        })
    }
    
    /// Decode Opus to PCM
    pub fn decode(&mut self, opus: &[u8]) -> Result<Vec<i16>, CodecError> {
        let mut output = vec![0i16; self.frame_size];
        
        let decoded_size = self.decoder
            .decode(Some(opus), &mut output, false)
            .map_err(|e| CodecError::DecoderError(format!("{:?}", e)))?;
        
        output.truncate(decoded_size);
        Ok(output)
    }
    
    /// Get frame size
    pub fn frame_size(&self) -> usize {
        self.frame_size
    }
}

impl Default for OpusEncoder {
    fn default() -> Self {
        Self::new().expect("Failed to create Opus encoder")
    }
}

impl Default for OpusDecoder {
    fn default() -> Self {
        Self::new().expect("Failed to create Opus decoder")
    }
}
