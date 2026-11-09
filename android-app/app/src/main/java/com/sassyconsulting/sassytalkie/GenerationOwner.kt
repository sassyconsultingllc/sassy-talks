// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-OM2SO5IDXQ3V
// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie

import java.util.concurrent.atomic.AtomicInteger

/** Monotonic ownership token for asynchronous transport lifecycles. */
internal class GenerationOwner {
    private val value = AtomicInteger(0)

    fun next(): Int = value.incrementAndGet()
    fun invalidate(): Int = value.incrementAndGet()
    fun current(): Int = value.get()
    fun owns(candidate: Int): Boolean = candidate == value.get()
}
