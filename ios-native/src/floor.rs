// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-IOSFLR2QW8VD
//! floor — iOS floor-control surface.
//!
//! Both the policy (who wins a simultaneous key-up, how long holds last) and
//! the runtime occupancy tracker (`FloorState`: owner, deadlines, reject
//! reason) live in `sassytalkie_core::floor`, shared with the desktop and
//! mirroring Android's PttCoordinator. This module keeps the user-facing
//! rejection strings, worded to match Android so support/QA see the same text.

pub use sassytalkie_core::floor::{
    FloorState, REJECT_CHANNEL_BUSY, REJECT_MAX_TX, REJECT_NOT_ENCRYPTED, REJECT_PREEMPTED,
};
