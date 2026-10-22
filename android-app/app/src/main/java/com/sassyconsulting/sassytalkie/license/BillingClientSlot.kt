// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie.license

/**
 * Holds the paywall's current billing client and swaps in a new one for every
 * connection attempt.
 *
 * Play Billing's `endConnection()` is terminal. The client moves to CLOSED, and
 * a later `startConnection()` on the same instance fails at once ("Client was
 * already closed and can't be reused", surfaced to the app as "Service
 * connection is disconnected."). From c4b604e (2026-07-08) the gate ended and
 * restarted one remembered client on every attempt, so the product never loaded
 * on any device and nobody could buy. Debug builds skip the gate, which is why
 * it went unseen; 3.2.7's changelog blamed sideloading.
 *
 * Generic so the rule can be tested without Play: a client handed out by
 * [fresh] is never one that has been ended, and each client is ended once.
 */
class BillingClientSlot<T : Any>(private val end: (T) -> Unit) {

    @Volatile
    var current: T? = null
        private set

    /** Ends the previous client (if any) and installs the one [create] builds. */
    @Synchronized
    fun fresh(create: () -> T): T {
        endCurrent()
        return create().also { current = it }
    }

    /** Ends the current client and leaves the slot empty. Safe to call twice. */
    @Synchronized
    fun close() {
        endCurrent()
    }

    private fun endCurrent() {
        val old = current ?: return
        current = null
        // A throwing endConnection() must not block the next attempt.
        runCatching { end(old) }
    }
}
