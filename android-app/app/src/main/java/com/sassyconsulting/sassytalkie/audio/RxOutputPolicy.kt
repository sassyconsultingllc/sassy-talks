// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-RXOUTPOL01
package com.sassyconsulting.sassytalkie.audio

/**
 * Sticky RX output policy. Walkie receive defaults to the loudspeaker —
 * never the earpiece — unless a real external device is connected or the
 * user explicitly chose earpiece in Settings.
 *
 * AudioDeviceInfo type integers are duplicated here so JVM unit tests do
 * not need the Android SDK. Values match [android.media.AudioDeviceInfo].
 */
object RxOutputPolicy {

    const val TYPE_BUILTIN_EARPIECE = 1
    const val TYPE_BUILTIN_SPEAKER = 2
    const val TYPE_WIRED_HEADSET = 3
    const val TYPE_WIRED_HEADPHONES = 4
    const val TYPE_LINE_ANALOG = 5
    const val TYPE_LINE_DIGITAL = 6
    const val TYPE_BLUETOOTH_SCO = 7
    const val TYPE_BLUETOOTH_A2DP = 8
    const val TYPE_HDMI = 9
    const val TYPE_USB_DEVICE = 11
    const val TYPE_USB_ACCESSORY = 12
    const val TYPE_DOCK = 13
    const val TYPE_FM = 14
    const val TYPE_BUILTIN_MIC = 15
    const val TYPE_FM_TUNER = 16
    const val TYPE_HDMI_ARC = 10
    const val TYPE_TELEPHONY = 18
    const val TYPE_AUX_LINE = 19
    const val TYPE_IP = 20
    const val TYPE_BUS = 21
    const val TYPE_USB_HEADSET = 22
    const val TYPE_HEARING_AID = 23
    const val TYPE_BUILTIN_SPEAKER_SAFE = 24
    const val TYPE_REMOTE_SUBMIX = 25
    const val TYPE_BLE_HEADSET = 26
    const val TYPE_BLE_SPEAKER = 27
    const val TYPE_HDMI_EARC = 29
    const val TYPE_BLE_BROADCAST = 30

    enum class Target {
        /** Built-in loudspeaker (walkie default). */
        LOUDSPEAKER,
        /** Built-in earpiece — only when the user opted in. */
        EARPIECE,
        /** Wired / BT / USB / hearing-aid — leave routing to the OS. */
        EXTERNAL,
    }

    fun isLoudspeakerType(type: Int): Boolean =
        type == TYPE_BUILTIN_SPEAKER || type == TYPE_BUILTIN_SPEAKER_SAFE

    fun isEarpieceType(type: Int): Boolean = type == TYPE_BUILTIN_EARPIECE

    /** True for a real accessory the user is listening through. Earpiece is not. */
    fun isExternalOutput(type: Int): Boolean = when (type) {
        TYPE_WIRED_HEADSET,
        TYPE_WIRED_HEADPHONES,
        TYPE_LINE_ANALOG,
        TYPE_LINE_DIGITAL,
        TYPE_BLUETOOTH_SCO,
        TYPE_BLUETOOTH_A2DP,
        TYPE_HDMI,
        TYPE_HDMI_ARC,
        TYPE_USB_DEVICE,
        TYPE_USB_ACCESSORY,
        TYPE_DOCK,
        TYPE_AUX_LINE,
        TYPE_IP,
        TYPE_USB_HEADSET,
        TYPE_HEARING_AID,
        TYPE_BLE_HEADSET,
        TYPE_BLE_SPEAKER,
        TYPE_HDMI_EARC,
        TYPE_BLE_BROADCAST,
        -> true
        else -> false
    }

    fun hasExternalOutput(outputTypes: IntArray): Boolean =
        outputTypes.any { isExternalOutput(it) }

    /**
     * Resolve the RX target.
     *
     * [userWantsSpeaker] is the Settings toggle (default true). An external
     * accessory always wins so we never steal a headset. Earpiece is never
     * chosen just because MODE_IN_COMMUNICATION defaulted there.
     */
    fun resolve(userWantsSpeaker: Boolean, outputTypes: IntArray): Target {
        if (hasExternalOutput(outputTypes)) return Target.EXTERNAL
        return if (userWantsSpeaker) Target.LOUDSPEAKER else Target.EARPIECE
    }

    /**
     * Whether a device switch is needed. Skip when already on the desired
     * sink so focus/packet events do not flap speaker ↔ earpiece.
     *
     * EXTERNAL: only act if we currently own a built-in sink (clear our
     * speaker/earpiece pin so the accessory can take over). If current is
     * already unknown or external, leave it alone.
     */
    fun shouldApply(target: Target, currentType: Int?): Boolean = when (target) {
        Target.LOUDSPEAKER -> currentType == null || !isLoudspeakerType(currentType)
        Target.EARPIECE -> currentType == null || !isEarpieceType(currentType)
        Target.EXTERNAL ->
            currentType != null && (isLoudspeakerType(currentType) || isEarpieceType(currentType))
    }
}
