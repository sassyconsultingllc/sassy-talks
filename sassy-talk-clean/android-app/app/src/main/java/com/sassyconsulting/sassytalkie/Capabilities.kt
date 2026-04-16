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
            val f = ControlFrame.decode(bytes)
            if (f.opcode != ControlFrame.OP_CAPABILITIES) return null
            return parse(f.payload)
        }
    }
}
