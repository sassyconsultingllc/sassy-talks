// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class GenerationOwnerTest {
    @Test
    fun `new connection invalidates every older callback`() {
        val owner = GenerationOwner()
        val first = owner.next()
        assertTrue(owner.owns(first))

        val second = owner.next()
        assertFalse(owner.owns(first))
        assertTrue(owner.owns(second))
    }

    @Test
    fun `disconnect invalidates active socket and reconnect task`() {
        val owner = GenerationOwner()
        val socket = owner.next()
        owner.invalidate()
        assertFalse(owner.owns(socket))
    }
}
