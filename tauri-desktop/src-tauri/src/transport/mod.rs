// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-7MQV4VWOFS4I
/// Transport Module - UDP Multicast for Cross-Platform Audio
///
/// Uses UDP multicast for automatic peer discovery and audio transmission
/// Works on WiFi networks (all desktop platforms)
///
/// Features:
/// - Random port selection per session for security
/// - End-to-end encryption with X25519 key exchange
/// - Configurable multicast address
///
/// Copyright 2025 Sassy Consulting LLC. All rights reserved.
pub mod cellular;
pub mod control;
pub mod control_plane;
pub mod discovery;
pub mod liveness;
pub mod manager;
pub mod tls_pinning;

pub use cellular::{CellularConfig, CellularState, CellularTransport};
pub use discovery::DiscoveryService;
pub use manager::{PeerInfo, TransportConfig, TransportManager};

use crate::constants;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

// Re-export constants for backwards compatibility
pub use constants::{
    BEACON_INTERVAL_SECS, DEFAULT_MULTICAST_ADDR as MULTICAST_ADDR,
    DEFAULT_MULTICAST_PORT as MULTICAST_PORT, MAX_PACKET_SIZE, PEER_TIMEOUT_SECS, PORT_RANGE_END,
    PORT_RANGE_START,
};

/// One received audio frame handed to the decode pipeline. Carries the sender
/// identity + wire timestamp so the RX side can key a PER-SENDER Opus decoder
/// (Opus is stateful) and order frames by their real timestamp, instead of
/// decoding every sender through one decoder in raw arrival order.
#[derive(Debug, Clone)]
pub struct AudioFrame {
    /// Wire `sender_id` (or a synthetic id for the legacy per-peer path).
    pub sender: String,
    /// Wire timestamp in ms; 0 when the transport carries none (legacy path),
    /// in which case the RX loop synthesizes a per-sender clock.
    pub timestamp: u64,
    /// Raw Opus payload.
    pub opus: Vec<u8>,
}

/// Snapshot of one bounded real-time queue. Counters are monotonic for the
/// lifetime of the transport and intentionally cheap enough for the audio path.
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct QueueMetricsSnapshot {
    pub enqueued: u64,
    pub dequeued: u64,
    pub dropped_full: u64,
    pub dropped_stale: u64,
    pub dropped_closed: u64,
}

#[derive(Debug, Default)]
pub struct QueueMetrics {
    enqueued: AtomicU64,
    dequeued: AtomicU64,
    dropped_full: AtomicU64,
    dropped_stale: AtomicU64,
    dropped_closed: AtomicU64,
}

impl QueueMetrics {
    pub fn snapshot(&self) -> QueueMetricsSnapshot {
        QueueMetricsSnapshot {
            enqueued: self.enqueued.load(Ordering::Relaxed),
            dequeued: self.dequeued.load(Ordering::Relaxed),
            dropped_full: self.dropped_full.load(Ordering::Relaxed),
            dropped_stale: self.dropped_stale.load(Ordering::Relaxed),
            dropped_closed: self.dropped_closed.load(Ordering::Relaxed),
        }
    }
}

struct Queued<T> {
    value: T,
    queued_at: Instant,
}

/// Sender for a bounded queue that never blocks a socket/audio producer.
pub struct RealtimeSender<T> {
    tx: mpsc::Sender<Queued<T>>,
    metrics: Arc<QueueMetrics>,
}

impl<T> Clone for RealtimeSender<T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            metrics: Arc::clone(&self.metrics),
        }
    }
}

impl<T> RealtimeSender<T> {
    /// Enqueue immediately or drop the newest item when saturated. Keeping the
    /// producer non-blocking prevents network bursts from delaying newer audio.
    pub fn try_send(&self, value: T) -> Result<(), &'static str> {
        match self.tx.try_send(Queued {
            value,
            queued_at: Instant::now(),
        }) {
            Ok(()) => {
                self.metrics.enqueued.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.metrics.dropped_full.fetch_add(1, Ordering::Relaxed);
                Err("real-time queue full")
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.metrics.dropped_closed.fetch_add(1, Ordering::Relaxed);
                Err("real-time queue closed")
            }
        }
    }

    pub fn metrics(&self) -> QueueMetricsSnapshot {
        self.metrics.snapshot()
    }
}

/// Receiver that silently skips items too old to be useful for live speech.
pub struct RealtimeReceiver<T> {
    rx: mpsc::Receiver<Queued<T>>,
    metrics: Arc<QueueMetrics>,
    max_age: Duration,
}

impl<T> RealtimeReceiver<T> {
    pub async fn recv(&mut self) -> Option<T> {
        while let Some(item) = self.rx.recv().await {
            if item.queued_at.elapsed() > self.max_age {
                self.metrics.dropped_stale.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            self.metrics.dequeued.fetch_add(1, Ordering::Relaxed);
            return Some(item.value);
        }
        None
    }

    /// Drain queued items, counting them as stale. Used after relay reconnect:
    /// replaying speech captured before the outage is worse than a short gap.
    pub fn discard_pending(&mut self) {
        while self.rx.try_recv().is_ok() {
            self.metrics.dropped_stale.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub fn realtime_channel<T>(
    capacity: usize,
    max_age: Duration,
) -> (RealtimeSender<T>, RealtimeReceiver<T>) {
    assert!(capacity > 0, "real-time queue capacity must be non-zero");
    let (tx, rx) = mpsc::channel(capacity);
    let metrics = Arc::new(QueueMetrics::default());
    (
        RealtimeSender {
            tx,
            metrics: Arc::clone(&metrics),
        },
        RealtimeReceiver {
            rx,
            metrics,
            max_age,
        },
    )
}

/// Transport error types
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to bind socket: {0}")]
    BindError(String),

    #[error("Failed to join multicast: {0}")]
    MulticastError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Invalid peer: {0}")]
    InvalidPeer(String),

    #[error("Encryption error: {0}")]
    EncryptionError(String),

    #[error("Key exchange failed: {0}")]
    KeyExchangeError(String),

    #[error("No port available in range")]
    NoPortAvailable,
}

#[cfg(test)]
mod queue_tests {
    use super::*;

    #[tokio::test]
    async fn bounded_queue_drops_when_full() {
        let (tx, mut rx) = realtime_channel(1, Duration::from_secs(1));
        assert!(tx.try_send(1).is_ok());
        assert_eq!(tx.try_send(2), Err("real-time queue full"));
        assert_eq!(rx.recv().await, Some(1));

        let metrics = tx.metrics();
        assert_eq!(metrics.enqueued, 1);
        assert_eq!(metrics.dequeued, 1);
        assert_eq!(metrics.dropped_full, 1);
    }

    #[tokio::test]
    async fn stale_items_are_skipped_and_counted() {
        let (tx, mut rx) = realtime_channel(2, Duration::from_millis(5));
        tx.try_send(1).unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        tx.try_send(2).unwrap();

        assert_eq!(rx.recv().await, Some(2));
        let metrics = tx.metrics();
        assert_eq!(metrics.dropped_stale, 1);
        assert_eq!(metrics.dequeued, 1);
    }
}
