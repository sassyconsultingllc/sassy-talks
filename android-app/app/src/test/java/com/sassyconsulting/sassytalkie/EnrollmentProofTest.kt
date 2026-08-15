// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

class EnrollmentProofTest {
    @Test
    fun `room id alone is not authorization`() {
        assertFalse(EnrollmentProof.joinAuthorized("room-id-long-enough", null, null, null))
        assertFalse(EnrollmentProof.joinAuthorized("room-id-long-enough", ByteArray(16), null, null))
        assertFalse(EnrollmentProof.joinAuthorized("short", ByteArray(32), null, null))
    }

    @Test
    fun `32-byte room secret authorizes when no enrollment token is required`() {
        assertTrue(EnrollmentProof.joinAuthorized("room-id-long-enough", ByteArray(32), null, null))
        assertTrue(EnrollmentProof.joinAuthorized("room-id-long-enough", ByteArray(32), "", ""))
    }

    @Test
    fun `mdm enrollment token mismatch is rejected`() {
        val secret = ByteArray(32) { 7 }
        assertFalse(
            EnrollmentProof.joinAuthorized(
                "room-id-long-enough",
                secret,
                "agency-token",
                "wrong-token-xx",
            ),
        )
        assertFalse(
            EnrollmentProof.joinAuthorized("room-id-long-enough", secret, "agency-token", null),
        )
        assertTrue(
            EnrollmentProof.joinAuthorized(
                "room-id-long-enough",
                secret,
                "agency-token",
                "agency-token",
            ),
        )
    }

    @Test
    fun `room secret proof round trips`() {
        val secret = ByteArray(32) { it.toByte() }
        val proof = EnrollmentProof.roomSecretProof(secret, "room-id-long-enough", "peer-a")
        assertNotNull(proof)
        assertTrue(
            EnrollmentProof.verifyRoomSecretProof(
                secret,
                "room-id-long-enough",
                "peer-a",
                proof!!,
            ),
        )
        assertFalse(
            EnrollmentProof.verifyRoomSecretProof(
                secret,
                "room-id-long-enough",
                "peer-b",
                proof,
            ),
        )
    }
}
