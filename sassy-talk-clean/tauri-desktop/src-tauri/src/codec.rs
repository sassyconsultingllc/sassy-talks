// Opus Codec - Low-latency voice compression
// Copyright 2025 Sassy Consulting LLC. All rights reserved.

use thiserror::Error;
use tracing::debug;
use audiopus::{coder::{Encoder, Decoder}, Channels, SampleRate, Application, Bitrate, MutSignals};

use crate::{FRAME_SIZE, OPUS_BITRATE};

#[derive(Error, Debug)]
pub enum CodecError {
    #[error("Encoder init failed: {0}")]
    EncoderInit(String),
    #[error("Decoder init failed: {0}")]
    DecoderInit(String),
    #[error("Encode failed: {0}")]
    EncodeFailed(String),
    #[error("Decode failed: {0}")]
    DecodeFailed(String),
    #[error("Invalid frame size")]
    InvalidFrameSize,
}

pub struct OpusEncoder {
    encoder: Encoder,
    frame_size: usize,
}

impl OpusEncoder {
    pub fn new() -> Result<Self, CodecError> {
        let encoder = Encoder::new(
            SampleRate::Hz48000,
            Channels::Mono,
            Application::Voip,
        )
        .map_err(|e| CodecError::EncoderInit(e.to_string()))?;
        
        let mut enc = Self {
            encoder,
            frame_size: FRAME_SIZE,
        };
        
        enc.configure()?;
        Ok(enc)
    }

    fn configure(&mut self) -> Result<(), CodecError> {
        self.encoder
            .set_bitrate(Bitrate::BitsPerSecond(OPUS_BITRATE))
            .map_err(|e| CodecError::EncoderInit(e.to_string()))?;
        
        self.encoder.set_vbr(true)
            .map_err(|e| CodecError::EncoderInit(e.to_string()))?;
        
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let complexity = 5;
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        let complexity = 8;
        
        self.encoder.set_complexity(complexity)
            .map_err(|e| CodecError::EncoderInit(e.to_string()))?;
        
        self.encoder.set_dtx(true)
            .map_err(|e| CodecError::EncoderInit(e.to_string()))?;
        
        self.encoder.set_inband_fec(true)
            .map_err(|e| CodecError::EncoderInit(e.to_string()))?;
        
        self.encoder.set_packet_loss_perc(5)
            .map_err(|e| CodecError::EncoderInit(e.to_string()))?;
        
        Ok(())
    }

    pub fn encode(&mut self, pcm: &[i16]) -> Result<Vec<u8>, CodecError> {
        if pcm.len() != self.frame_size {
            return Err(CodecError::InvalidFrameSize);
        }
        
        let mut output = vec![0u8; 1275];
        
        let len = self.encoder
            .encode(pcm, &mut output)
            .map_err(|e| CodecError::EncodeFailed(e.to_string()))?;
        
        output.truncate(len);
        debug!("Encoded {} samples -> {} bytes", pcm.len(), len);
        
        Ok(output)
    }
}

pub struct OpusDecoder {
    decoder: Decoder,
    frame_size: usize,
}

impl OpusDecoder {
    pub fn new() -> Result<Self, CodecError> {
        let decoder = Decoder::new(
            SampleRate::Hz48000,
            Channels::Mono,
        )
        .map_err(|e| CodecError::DecoderInit(e.to_string()))?;
        
        Ok(Self {
            decoder,
            frame_size: FRAME_SIZE,
        })
    }

    pub fn decode(&mut self, opus_data: &[u8]) -> Result<Vec<i16>, CodecError> {
        let mut output = vec![0i16; self.frame_size];
        
        let mut_signals = MutSignals::try_from(&mut output[..])
            .map_err(|e| CodecError::DecodeFailed(e.to_string()))?;
        
        let len = self.decoder
     .decode(Some(audiopus::packet::Packet::try_from(opus_data).map_err(|e| CodecError::DecodeFailed(e.to_string()))?), mut_signals, false)
            .map_err(|e| CodecError::DecodeFailed(e.to_string()))?;
        
        output.truncate(len);
        debug!("Decoded {} bytes -> {} samples", opus_data.len(), len);
        
        Ok(output)
    }

    pub fn decode_missing(&mut self) -> Result<Vec<i16>, CodecError> {
        let mut output = vec![0i16; self.frame_size];
        
        let mut_signals = MutSignals::try_from(&mut output[..])
            .map_err(|e| CodecError::DecodeFailed(e.to_string()))?;
        
        let len = self.decoder
            .decode(None, mut_signals, false)
            .map_err(|e| CodecError::DecodeFailed(e.to_string()))?;
        
        output.truncate(len);
        
        Ok(output)
    }

    pub fn reset(&mut self) -> Result<(), CodecError> {
        self.decoder = Decoder::new(
            SampleRate::Hz48000,
            Channels::Mono,
        )
        .map_err(|e| CodecError::DecoderInit(e.to_string()))?;
        
        Ok(())
    }
}