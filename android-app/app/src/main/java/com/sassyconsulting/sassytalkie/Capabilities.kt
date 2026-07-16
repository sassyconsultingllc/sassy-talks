// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-Z65MSNOSPLKQ
package com.sassyconsulting.sassytalkie

import org.json.JSONObject

data class Capabilities(
    val codec: String,
    val sampleRate: Int,
    val mute: Boolean,
    val vol: Int,
    val battery: Int,
    val audioV2: Boolean,
    val epoch: Long,
) {
    fun toFrame(): ByteArray {
        val json = JSONObject()
            .put("codec", codec)
            .put("sample_rate", sampleRate)
            .put("mute", mute)
            .put("vol", vol)
            .put("battery", battery)
            .put("audio_v2", audioV2)
            .put("epoch", epoch)
            .toString()
            .toByteArray(Charsets.UTF_8)
        return ControlFrame.encodeTlv(ControlFrame.OP_CAPABILITIES, json)
    }

    companion object {
        fun parse(payload: ByteArray): Capabilities {
            require(payload.size <= 4096) { "capabilities payload too large: ${payload.size} bytes" }
            val o = JSONObject(String(payload, Charsets.UTF_8))
            return Capabilities(
                codec = o.optString("codec", "opus"),
                sampleRate = o.optInt("sample_rate", 24000),
                mute = o.optBoolean("mute", false),
                vol = o.optInt("vol", 100),
                battery = o.optInt("battery", -1),
                audioV2 = o.optBoolean("audio_v2", false),
                epoch = o.optLong("epoch", 0L),
            )
        }

        fun fromFrame(bytes: ByteArray): Capabilities? {
            val f = ControlFrame.decode(bytes) ?: return null
            if (f.opcode != ControlFrame.OP_CAPABILITIES) return null
            return parse(f.payload)
        }
    }
}
