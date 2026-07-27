// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-LTT8K2M9QXWP
package com.sassyconsulting.sassytalkie.translate

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LiveTranslationLifecycleTest {

    @Test
    fun `disabled does not start on acquire`() {
        val life = LiveTranslationLifecycle()
        assertEquals(LiveTranslationLifecycle.MicAction.NONE, life.acquireUi())
        assertFalse(life.shouldRun(pttActive = false))
    }

    @Test
    fun `enabled with ui consumer starts`() {
        val life = LiveTranslationLifecycle()
        life.acquireUi()
        assertEquals(LiveTranslationLifecycle.MicAction.START, life.setEnabled(true))
        assertTrue(life.shouldRun(pttActive = false))
    }

    @Test
    fun `setEnabled false stops and clears pause`() {
        val life = LiveTranslationLifecycle()
        life.acquireUi()
        life.setEnabled(true)
        life.onPttStarted()
        assertTrue(life.pausedForPtt)
        assertEquals(LiveTranslationLifecycle.MicAction.STOP, life.setEnabled(false))
        assertFalse(life.enabled)
        assertFalse(life.pausedForPtt)
        assertFalse(life.shouldRun(pttActive = false))
    }

    @Test
    fun `PTT pause stops and resume restarts`() {
        val life = LiveTranslationLifecycle()
        life.acquireUi()
        life.setEnabled(true)
        assertEquals(LiveTranslationLifecycle.MicAction.STOP, life.onPttStarted())
        assertTrue(life.pausedForPtt)
        assertFalse(life.shouldRun(pttActive = false))

        assertEquals(
            LiveTranslationLifecycle.MicAction.START,
            life.onPttResumeReady(pttStillActive = false),
        )
        assertFalse(life.pausedForPtt)
        assertTrue(life.shouldRun(pttActive = false))
    }

    @Test
    fun `duplicate PTT start is idempotent`() {
        val life = LiveTranslationLifecycle()
        life.acquireUi()
        life.setEnabled(true)
        assertEquals(LiveTranslationLifecycle.MicAction.STOP, life.onPttStarted())
        assertEquals(LiveTranslationLifecycle.MicAction.NONE, life.onPttStarted())
        assertTrue(life.pausedForPtt)
    }

    @Test
    fun `resume while PTT still active stays paused`() {
        val life = LiveTranslationLifecycle()
        life.acquireUi()
        life.setEnabled(true)
        life.onPttStarted()
        assertEquals(
            LiveTranslationLifecycle.MicAction.NONE,
            life.onPttResumeReady(pttStillActive = true),
        )
        assertTrue(life.pausedForPtt)
    }

    @Test
    fun `ui consumer refcount stops at zero`() {
        val life = LiveTranslationLifecycle()
        life.setEnabled(true)
        life.acquireUi()
        life.acquireUi()
        assertEquals(LiveTranslationLifecycle.MicAction.NONE, life.releaseUi())
        assertEquals(1, life.uiConsumers)
        assertEquals(LiveTranslationLifecycle.MicAction.STOP, life.releaseUi())
        assertEquals(0, life.uiConsumers)
        assertFalse(life.shouldRun(pttActive = false))
    }

    @Test
    fun `releaseUi never goes negative`() {
        val life = LiveTranslationLifecycle()
        life.releaseUi()
        life.releaseUi()
        assertEquals(0, life.uiConsumers)
    }

    @Test
    fun `pttActive blocks shouldRun even when not paused flag`() {
        val life = LiveTranslationLifecycle()
        life.acquireUi()
        life.setEnabled(true)
        assertFalse(life.shouldRun(pttActive = true))
    }

    @Test
    fun `disable stops shouldRun even with leftover ui consumers`() {
        val life = LiveTranslationLifecycle()
        life.acquireUi()
        life.acquireUi()
        life.setEnabled(true)
        assertTrue(life.shouldRun(pttActive = false))
        assertEquals(LiveTranslationLifecycle.MicAction.STOP, life.setEnabled(false))
        assertEquals(2, life.uiConsumers)
        assertFalse(life.shouldRun(pttActive = false))
    }
}

class LiveTranslationTextTest {

    @Test
    fun `timelineEntry blank caption is null`() {
        assertNull(LiveTranslationText.timelineEntry("  ", "hola"))
    }

    @Test
    fun `timelineEntry same text returns caption only`() {
        assertEquals("hello", LiveTranslationText.timelineEntry("hello", "Hello"))
    }

    @Test
    fun `timelineEntry pairs translation with source`() {
        assertEquals("hola\n(hello)", LiveTranslationText.timelineEntry("hello", "hola"))
    }

    @Test
    fun `normalizeKey collapses whitespace and case`() {
        assertEquals(
            "hello world",
            LiveTranslationText.normalizeKey("  Hello   WORLD  "),
        )
    }

    @Test
    fun `shouldSpeakTts respects PTT and incoming audio`() {
        assertTrue(LiveTranslationText.shouldSpeakTts(true, false, false))
        assertFalse(LiveTranslationText.shouldSpeakTts(false, false, false))
        assertFalse(LiveTranslationText.shouldSpeakTts(true, true, false))
        assertFalse(LiveTranslationText.shouldSpeakTts(true, false, true))
    }

    @Test
    fun `speakableFromTimeline prefers translated line`() {
        assertEquals(
            "Hola",
            LiveTranslationText.speakableFromTimeline("Hola\n(Hello)"),
        )
        assertEquals(
            "just caption",
            LiveTranslationText.speakableFromTimeline("just caption"),
        )
        assertEquals("", LiveTranslationText.speakableFromTimeline("   "))
    }

    @Test
    fun `downloadStatusLine names the language pair`() {
        val line = LiveTranslationText.downloadStatusLine("en", "es")
        assertTrue(line.contains("→"))
        assertTrue(line.contains("~30 MB"))
        assertTrue(line.contains("English") || line.contains("en"))
    }
}
