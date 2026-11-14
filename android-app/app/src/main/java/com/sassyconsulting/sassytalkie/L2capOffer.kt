// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * What a device publishes on the L2CAP GATT characteristic so a peer can open
 * the Bluetooth audio channel without pairing.
 *
 * Wire layout (10 bytes, little-endian): `[psm:u16][nonce:u64]`.
 *
 * - `psm` is the dynamic LE L2CAP channel number the device is listening on
 *   (0 = not listening; the peer falls back to classic RFCOMM).
 * - `nonce` is random per process and only breaks the symmetric-dial tie: both
 *   phones discover each other and both could dial, which leaves two sockets
 *   and each side closing the one the other kept. The lower nonce dials.
 */
internal data class L2capOffer(val psm: Int, val nonce: Long) {

    fun encode(): ByteArray = ByteBuffer.allocate(WIRE_BYTES).order(ByteOrder.LITTLE_ENDIAN)
        .putShort(psm.toShort())
        .putLong(nonce)
        .array()

    /** True when this offer names a channel a peer can actually dial. */
    val dialable: Boolean get() = psm in 1..MAX_PSM

    companion object {
        const val WIRE_BYTES = 10
        // 0 means "not listening". Android hands out dynamic LE PSMs in
        // 0x0080..0x00FF, but the field is 16 bits and a failed dial is cheap,
        // so any non-zero value is worth trying rather than second-guessing
        // a vendor stack.
        const val MAX_PSM = 0xFFFF

        fun decode(bytes: ByteArray?): L2capOffer? {
            if (bytes == null || bytes.size < WIRE_BYTES) return null
            val b = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
            return L2capOffer(b.short.toInt() and 0xFFFF, b.long)
        }

        /**
         * True if the local side should open the channel. Unsigned compare so
         * the answer is the mirror image on the two devices; equal nonces
         * (2^-64) make both dial and the duplicate-socket guard sorts it out.
         */
        fun localDials(localNonce: Long, peerNonce: Long): Boolean =
            java.lang.Long.compareUnsigned(localNonce, peerNonce) <= 0
    }
}
