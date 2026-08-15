// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-RXRPOLICY01
//! Sticky RX output policy (pure). Walkie receive defaults to the
//! loudspeaker — never the earpiece — unless a real external accessory is
//! connected or the user explicitly chose earpiece.
//!
//! Type integers match `android.media.AudioDeviceInfo`.

pub const TYPE_BUILTIN_EARPIECE: i32 = 1;
pub const TYPE_BUILTIN_SPEAKER: i32 = 2;
pub const TYPE_WIRED_HEADSET: i32 = 3;
pub const TYPE_WIRED_HEADPHONES: i32 = 4;
pub const TYPE_LINE_ANALOG: i32 = 5;
pub const TYPE_LINE_DIGITAL: i32 = 6;
pub const TYPE_BLUETOOTH_SCO: i32 = 7;
pub const TYPE_BLUETOOTH_A2DP: i32 = 8;
pub const TYPE_HDMI: i32 = 9;
pub const TYPE_HDMI_ARC: i32 = 10;
pub const TYPE_USB_DEVICE: i32 = 11;
pub const TYPE_USB_ACCESSORY: i32 = 12;
pub const TYPE_DOCK: i32 = 13;
pub const TYPE_USB_HEADSET: i32 = 22;
pub const TYPE_HEARING_AID: i32 = 23;
pub const TYPE_BUILTIN_SPEAKER_SAFE: i32 = 24;
pub const TYPE_BLE_HEADSET: i32 = 26;
pub const TYPE_BLE_SPEAKER: i32 = 27;
pub const TYPE_HDMI_EARC: i32 = 29;
pub const TYPE_BLE_BROADCAST: i32 = 30;
pub const TYPE_AUX_LINE: i32 = 19;
pub const TYPE_IP: i32 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Loudspeaker,
    Earpiece,
    External,
}

pub fn is_loudspeaker_type(t: i32) -> bool {
    t == TYPE_BUILTIN_SPEAKER || t == TYPE_BUILTIN_SPEAKER_SAFE
}

pub fn is_earpiece_type(t: i32) -> bool {
    t == TYPE_BUILTIN_EARPIECE
}

pub fn is_external_output(t: i32) -> bool {
    matches!(
        t,
        TYPE_WIRED_HEADSET
            | TYPE_WIRED_HEADPHONES
            | TYPE_LINE_ANALOG
            | TYPE_LINE_DIGITAL
            | TYPE_BLUETOOTH_SCO
            | TYPE_BLUETOOTH_A2DP
            | TYPE_HDMI
            | TYPE_HDMI_ARC
            | TYPE_USB_DEVICE
            | TYPE_USB_ACCESSORY
            | TYPE_DOCK
            | TYPE_AUX_LINE
            | TYPE_IP
            | TYPE_USB_HEADSET
            | TYPE_HEARING_AID
            | TYPE_BLE_HEADSET
            | TYPE_BLE_SPEAKER
            | TYPE_HDMI_EARC
            | TYPE_BLE_BROADCAST
    )
}

pub fn has_external_output(types: &[i32]) -> bool {
    types.iter().copied().any(is_external_output)
}

pub fn resolve(user_wants_speaker: bool, output_types: &[i32]) -> Target {
    if has_external_output(output_types) {
        Target::External
    } else if user_wants_speaker {
        Target::Loudspeaker
    } else {
        Target::Earpiece
    }
}

pub fn should_apply(target: Target, current_type: Option<i32>) -> bool {
    match target {
        Target::Loudspeaker => current_type
            .map(|t| !is_loudspeaker_type(t))
            .unwrap_or(true),
        Target::Earpiece => current_type.map(|t| !is_earpiece_type(t)).unwrap_or(true),
        Target::External => current_type
            .map(|t| is_loudspeaker_type(t) || is_earpiece_type(t))
            .unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_rx_is_loudspeaker_not_earpiece() {
        assert_eq!(
            resolve(true, &[TYPE_BUILTIN_EARPIECE, TYPE_BUILTIN_SPEAKER]),
            Target::Loudspeaker
        );
    }

    #[test]
    fn explicit_earpiece_when_no_headset() {
        assert_eq!(
            resolve(false, &[TYPE_BUILTIN_EARPIECE, TYPE_BUILTIN_SPEAKER]),
            Target::Earpiece
        );
    }

    #[test]
    fn bluetooth_sco_and_a2dp_win() {
        assert_eq!(
            resolve(true, &[TYPE_BUILTIN_SPEAKER, TYPE_BLUETOOTH_SCO]),
            Target::External
        );
        assert_eq!(
            resolve(true, &[TYPE_BUILTIN_SPEAKER, TYPE_BLUETOOTH_A2DP]),
            Target::External
        );
    }

    #[test]
    fn wired_and_usb_are_external() {
        assert_eq!(
            resolve(true, &[TYPE_BUILTIN_SPEAKER, TYPE_WIRED_HEADSET]),
            Target::External
        );
        assert!(is_external_output(TYPE_USB_HEADSET));
        assert!(!is_external_output(TYPE_BUILTIN_EARPIECE));
    }

    #[test]
    fn no_flap_when_already_on_target() {
        assert!(!should_apply(
            Target::Loudspeaker,
            Some(TYPE_BUILTIN_SPEAKER)
        ));
        assert!(!should_apply(
            Target::Loudspeaker,
            Some(TYPE_BUILTIN_SPEAKER_SAFE)
        ));
        assert!(should_apply(
            Target::Loudspeaker,
            Some(TYPE_BUILTIN_EARPIECE)
        ));
        assert!(should_apply(Target::Loudspeaker, None));
        assert!(!should_apply(Target::Earpiece, Some(TYPE_BUILTIN_EARPIECE)));
        assert!(should_apply(Target::External, Some(TYPE_BUILTIN_SPEAKER)));
        assert!(!should_apply(Target::External, Some(TYPE_BLUETOOTH_SCO)));
        assert!(!should_apply(Target::External, None));
    }
}
