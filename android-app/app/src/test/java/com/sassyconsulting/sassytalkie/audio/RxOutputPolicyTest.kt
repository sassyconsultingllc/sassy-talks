// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-RXOUTPOL02
package com.sassyconsulting.sassytalkie.audio

import com.sassyconsulting.sassytalkie.audio.RxOutputPolicy.TYPE_BLE_HEADSET
import com.sassyconsulting.sassytalkie.audio.RxOutputPolicy.TYPE_BLUETOOTH_A2DP
import com.sassyconsulting.sassytalkie.audio.RxOutputPolicy.TYPE_BLUETOOTH_SCO
import com.sassyconsulting.sassytalkie.audio.RxOutputPolicy.TYPE_BUILTIN_EARPIECE
import com.sassyconsulting.sassytalkie.audio.RxOutputPolicy.TYPE_BUILTIN_SPEAKER
import com.sassyconsulting.sassytalkie.audio.RxOutputPolicy.TYPE_BUILTIN_SPEAKER_SAFE
import com.sassyconsulting.sassytalkie.audio.RxOutputPolicy.TYPE_HEARING_AID
import com.sassyconsulting.sassytalkie.audio.RxOutputPolicy.TYPE_USB_HEADSET
import com.sassyconsulting.sassytalkie.audio.RxOutputPolicy.TYPE_WIRED_HEADPHONES
import com.sassyconsulting.sassytalkie.audio.RxOutputPolicy.TYPE_WIRED_HEADSET
import com.sassyconsulting.sassytalkie.audio.RxOutputPolicy.Target
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RxOutputPolicyTest {

    private fun types(vararg t: Int) = t

    @Test
    fun defaultRxIsLoudspeakerNotEarpiece() {
        assertEquals(
            Target.LOUDSPEAKER,
            RxOutputPolicy.resolve(
                userWantsSpeaker = true,
                outputTypes = types(TYPE_BUILTIN_EARPIECE, TYPE_BUILTIN_SPEAKER),
            ),
        )
    }

    @Test
    fun explicitEarpiecePreferenceHonoredWhenNoHeadset() {
        assertEquals(
            Target.EARPIECE,
            RxOutputPolicy.resolve(
                userWantsSpeaker = false,
                outputTypes = types(TYPE_BUILTIN_EARPIECE, TYPE_BUILTIN_SPEAKER),
            ),
        )
    }

    @Test
    fun bluetoothScoOverridesSpeakerPreference() {
        assertEquals(
            Target.EXTERNAL,
            RxOutputPolicy.resolve(
                userWantsSpeaker = true,
                outputTypes = types(TYPE_BUILTIN_SPEAKER, TYPE_BLUETOOTH_SCO),
            ),
        )
    }

    @Test
    fun bluetoothA2dpOverridesSpeakerPreference() {
        assertEquals(
            Target.EXTERNAL,
            RxOutputPolicy.resolve(
                userWantsSpeaker = true,
                outputTypes = types(TYPE_BUILTIN_SPEAKER, TYPE_BLUETOOTH_A2DP),
            ),
        )
    }

    @Test
    fun wiredHeadsetAndHeadphonesAreExternal() {
        assertEquals(
            Target.EXTERNAL,
            RxOutputPolicy.resolve(true, types(TYPE_BUILTIN_SPEAKER, TYPE_WIRED_HEADSET)),
        )
        assertEquals(
            Target.EXTERNAL,
            RxOutputPolicy.resolve(true, types(TYPE_BUILTIN_SPEAKER, TYPE_WIRED_HEADPHONES)),
        )
    }

    @Test
    fun usbHeadsetAndHearingAidAndBleHeadsetAreExternal() {
        assertTrue(RxOutputPolicy.isExternalOutput(TYPE_USB_HEADSET))
        assertTrue(RxOutputPolicy.isExternalOutput(TYPE_HEARING_AID))
        assertTrue(RxOutputPolicy.isExternalOutput(TYPE_BLE_HEADSET))
        assertEquals(
            Target.EXTERNAL,
            RxOutputPolicy.resolve(true, types(TYPE_BUILTIN_SPEAKER, TYPE_USB_HEADSET)),
        )
    }

    @Test
    fun earpieceIsNotAnExternalOverride() {
        assertFalse(RxOutputPolicy.isExternalOutput(TYPE_BUILTIN_EARPIECE))
        assertFalse(RxOutputPolicy.hasExternalOutput(types(TYPE_BUILTIN_EARPIECE, TYPE_BUILTIN_SPEAKER)))
    }

    @Test
    fun shouldNotFlapWhenAlreadyOnLoudspeaker() {
        assertFalse(RxOutputPolicy.shouldApply(Target.LOUDSPEAKER, TYPE_BUILTIN_SPEAKER))
        assertFalse(RxOutputPolicy.shouldApply(Target.LOUDSPEAKER, TYPE_BUILTIN_SPEAKER_SAFE))
    }

    @Test
    fun shouldSwitchEarpieceToLoudspeaker() {
        assertTrue(RxOutputPolicy.shouldApply(Target.LOUDSPEAKER, TYPE_BUILTIN_EARPIECE))
        assertTrue(RxOutputPolicy.shouldApply(Target.LOUDSPEAKER, null))
    }

    @Test
    fun shouldNotFlapWhenAlreadyOnEarpiecePreference() {
        assertFalse(RxOutputPolicy.shouldApply(Target.EARPIECE, TYPE_BUILTIN_EARPIECE))
        assertTrue(RxOutputPolicy.shouldApply(Target.EARPIECE, TYPE_BUILTIN_SPEAKER))
    }

    @Test
    fun externalClearsBuiltinPinButDoesNotTouchHeadset() {
        assertTrue(RxOutputPolicy.shouldApply(Target.EXTERNAL, TYPE_BUILTIN_SPEAKER))
        assertTrue(RxOutputPolicy.shouldApply(Target.EXTERNAL, TYPE_BUILTIN_EARPIECE))
        assertFalse(RxOutputPolicy.shouldApply(Target.EXTERNAL, TYPE_BLUETOOTH_SCO))
        assertFalse(RxOutputPolicy.shouldApply(Target.EXTERNAL, TYPE_WIRED_HEADSET))
        assertFalse(RxOutputPolicy.shouldApply(Target.EXTERNAL, null))
    }
}
