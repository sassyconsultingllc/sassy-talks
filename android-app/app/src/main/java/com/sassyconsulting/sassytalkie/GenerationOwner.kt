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
