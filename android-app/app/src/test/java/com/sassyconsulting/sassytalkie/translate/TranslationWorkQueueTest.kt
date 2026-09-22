// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie.translate

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class TranslationWorkQueueTest {
    @Test
    fun `partials conflate to latest hypothesis`() {
        val queue = TranslationWorkQueue()
        queue.offer("hel", false)
        queue.offer("hello", false)

        assertEquals(TranslationWorkQueue.Work("hello", false), queue.poll())
        assertNull(queue.poll())
    }

    @Test
    fun `final removes stale partial and has priority`() {
        val queue = TranslationWorkQueue()
        queue.offer("stale partial", false)
        queue.offer("first final", true)
        queue.offer("second final", true)
        queue.offer("new partial", false)

        assertEquals(TranslationWorkQueue.Work("first final", true), queue.poll())
        assertEquals(TranslationWorkQueue.Work("second final", true), queue.poll())
        assertEquals(TranslationWorkQueue.Work("new partial", false), queue.poll())
    }

    @Test
    fun `stopping recognition clears partial but preserves final`() {
        val queue = TranslationWorkQueue()
        queue.offer("final", true)
        queue.offer("partial", false)
        queue.clearPartial()

        assertEquals(TranslationWorkQueue.Work("final", true), queue.poll())
        assertNull(queue.poll())
    }
}
