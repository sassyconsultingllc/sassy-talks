// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie.license

import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

/**
 * Holds back [onDone] until every purchase acknowledgement in flight has
 * called back.
 *
 * `acknowledgePurchase` runs on the billing client's own executor, and
 * `endConnection()` calls `shutdownNow()` on that executor. Ending the
 * connection right after starting an ack therefore drops the ack silently, and
 * Play refunds a purchase left unacknowledged for 3 days. `refresh()` is the
 * only place that acknowledges purchases which completed while the app was
 * closed, so it has to wait for the callbacks before it ends the connection.
 *
 * Use: [begin] once per ack and call the returned function from that ack's
 * callback; [seal] once every ack has been started. [onDone] runs exactly
 * once: when the last ack calls back after [seal], or at [seal] if nothing is
 * pending, or at [forceDone] (the timeout backstop), whichever comes first.
 */
class PendingAcks(private val onDone: () -> Unit) {
    private val pending = AtomicInteger(0)
    private val sealed = AtomicBoolean(false)
    private val fired = AtomicBoolean(false)

    /** Registers one ack. The returned function counts only its first call. */
    fun begin(): () -> Unit {
        pending.incrementAndGet()
        val called = AtomicBoolean(false)
        return {
            if (called.compareAndSet(false, true) &&
                pending.decrementAndGet() == 0 &&
                sealed.get()
            ) {
                fire()
            }
        }
    }

    /** No more acks will be started; finish as soon as none are pending. */
    fun seal() {
        sealed.set(true)
        if (pending.get() == 0) fire()
    }

    /** Timeout backstop: finish now even if Play never called back. */
    fun forceDone() = fire()

    private fun fire() {
        if (fired.compareAndSet(false, true)) onDone()
    }
}
