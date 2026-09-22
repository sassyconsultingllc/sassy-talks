// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/**
 * Room ID is not authorization. Join requires a 32-byte room secret (PSK)
 * and, when MDM supplies [ManagedConfig.KEY_ENROLLMENT_TOKEN], that token.
 */
object EnrollmentProof {
    private const val DOMAIN = "sassytalkie-enroll-v1"

    fun joinAuthorized(
        roomId: String,
        roomSecret: ByteArray?,
        requiredEnrollmentToken: String?,
        presentedEnrollmentToken: String?,
    ): Boolean {
        if (roomId.length < 8) return false
        if (roomSecret == null || roomSecret.size != 32) return false
        return enrollmentTokenAcceptable(requiredEnrollmentToken, presentedEnrollmentToken)
    }

    fun enrollmentTokenAcceptable(required: String?, presented: String?): Boolean {
        val need = required?.trim().orEmpty()
        if (need.isEmpty()) return true
        val got = presented?.trim().orEmpty()
        if (got.isEmpty() || got.length != need.length) return false
        var diff = 0
        for (i in need.indices) diff = diff or (need[i].code xor got[i].code)
        return diff == 0
    }

    fun roomSecretProof(roomSecret: ByteArray, roomId: String, peerId: String): String? {
        if (roomSecret.size != 32 || roomId.length < 8 || peerId.isEmpty()) return null
        return try {
            val mac = Mac.getInstance("HmacSHA256")
            mac.init(SecretKeySpec(roomSecret, "HmacSHA256"))
            mac.update(DOMAIN.toByteArray(Charsets.UTF_8))
            mac.update(0)
            mac.update(roomId.toByteArray(Charsets.UTF_8))
            mac.update(0)
            mac.update(peerId.toByteArray(Charsets.UTF_8))
            mac.doFinal().joinToString("") { "%02x".format(it) }
        } catch (_: Throwable) {
            null
        }
    }

    fun verifyRoomSecretProof(
        roomSecret: ByteArray,
        roomId: String,
        peerId: String,
        presented: String,
    ): Boolean {
        val expected = roomSecretProof(roomSecret, roomId, peerId) ?: return false
        if (expected.length != presented.trim().length) return false
        var diff = 0
        val a = expected
        val b = presented.trim()
        for (i in a.indices) diff = diff or (a[i].code xor b[i].code)
        return diff == 0
    }
}
