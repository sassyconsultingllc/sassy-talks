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

    @Test
    fun `incoming peer audio pauses captioning`() {
        val life = LiveTranslationLifecycle()
        life.acquireUi()
        life.setEnabled(true)
        assertTrue(life.shouldRun(pttActive = false))
        assertEquals(LiveTranslationLifecycle.MicAction.STOP, life.onIncomingStarted())
        assertTrue(life.pausedForIncoming)
        assertFalse(life.shouldRun(pttActive = false))
        assertEquals(
            LiveTranslationLifecycle.MicAction.START,
            life.onIncomingEnded(),
        )
        assertFalse(life.pausedForIncoming)
        assertTrue(life.shouldRun(pttActive = false))
    }

    @Test
    fun `duplicate incoming start is idempotent`() {
        val life = LiveTranslationLifecycle()
        life.acquireUi()
        life.setEnabled(true)
        assertEquals(LiveTranslationLifecycle.MicAction.STOP, life.onIncomingStarted())
        assertEquals(LiveTranslationLifecycle.MicAction.NONE, life.onIncomingStarted())
        assertTrue(life.pausedForIncoming)
    }

    @Test
    fun `PTT resume while peer is speaking stays paused`() {
        val life = LiveTranslationLifecycle()
        life.acquireUi()
        life.setEnabled(true)
        life.onPttStarted()
        life.onIncomingStarted()
        assertEquals(
            LiveTranslationLifecycle.MicAction.NONE,
            life.onPttResumeReady(pttStillActive = false),
        )
        assertFalse(life.pausedForPtt)
        assertTrue(life.pausedForIncoming)
        assertFalse(life.shouldRun(pttActive = false))
    }

    @Test
    fun `incoming end while PTT paused does not start`() {
        val life = LiveTranslationLifecycle()
        life.acquireUi()
        life.setEnabled(true)
        life.onPttStarted()
        life.onIncomingStarted()
        assertEquals(
            LiveTranslationLifecycle.MicAction.NONE,
            life.onIncomingEnded(),
        )
        assertFalse(life.pausedForIncoming)
        assertTrue(life.pausedForPtt)
        assertFalse(life.shouldRun(pttActive = false))
    }

    @Test
    fun `setEnabled false clears incoming pause`() {
        val life = LiveTranslationLifecycle()
        life.acquireUi()
        life.setEnabled(true)
        life.onIncomingStarted()
        assertTrue(life.pausedForIncoming)
        assertEquals(LiveTranslationLifecycle.MicAction.STOP, life.setEnabled(false))
        assertFalse(life.pausedForIncoming)
    }

    @Test
    fun `incoming end is no-op when not paused`() {
        val life = LiveTranslationLifecycle()
        life.acquireUi()
        life.setEnabled(true)
        assertEquals(LiveTranslationLifecycle.MicAction.NONE, life.onIncomingEnded())
        assertTrue(life.shouldRun(pttActive = false))
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
    fun `canSpeakNow refuses TX RX and disabled feature`() {
        assertTrue(LiveTranslationText.canSpeakNow(true, true, false, false, false))
        assertFalse(LiveTranslationText.canSpeakNow(false, true, false, false, false))
        assertFalse(LiveTranslationText.canSpeakNow(true, false, false, false, false))
        assertFalse(LiveTranslationText.canSpeakNow(true, true, true, false, false))
        assertFalse(LiveTranslationText.canSpeakNow(true, true, false, true, false))
        assertFalse(LiveTranslationText.canSpeakNow(true, true, false, false, true))
    }

    @Test
    fun `shouldQueueTts when blocked by PTT or incoming`() {
        assertTrue(LiveTranslationText.shouldQueueTts(true, true, false))
        assertTrue(LiveTranslationText.shouldQueueTts(true, false, true))
        assertTrue(LiveTranslationText.shouldQueueTts(true, true, true))
        assertFalse(LiveTranslationText.shouldQueueTts(true, false, false))
        assertFalse(LiveTranslationText.shouldQueueTts(false, true, false))
    }

    @Test
    fun `speakableUtterance prefers translation`() {
        assertEquals("hola", LiveTranslationText.speakableUtterance("hello", "hola"))
        assertEquals("hello", LiveTranslationText.speakableUtterance("hello", "  "))
        assertEquals("", LiveTranslationText.speakableUtterance("  ", ""))
    }

    @Test
    fun `needsOfflineSpeechPack detects pack errors`() {
        assertTrue(LiveTranslationText.needsOfflineSpeechPack("Offline language model not installed"))
        assertTrue(LiveTranslationText.needsOfflineSpeechPack("Language not supported offline"))
        assertTrue(LiveTranslationText.needsOfflineSpeechPack("error 12"))
        assertTrue(LiveTranslationText.needsOfflineSpeechPack("Recognition error (12)"))
        assertTrue(LiveTranslationText.needsOfflineSpeechPack("Language not supported offline (error 12)"))
        assertFalse(LiveTranslationText.needsOfflineSpeechPack("Recognizer busy (PTT using mic?)"))
        assertFalse(LiveTranslationText.needsOfflineSpeechPack(null))
    }

    @Test
    fun `radio overlay prefers speech pack over infinite download`() {
        val downloadingButPackMissing = LiveTranslationText.radioOverlayPrimary(
            pausedForPtt = false,
            ttsEnabled = false,
            modelDownloading = true,
            needsSpeechPack = true,
            statusError = false,
            translation = "",
            caption = "",
            listening = false,
            sourceCode = "en",
            targetCode = "es",
        )
        assertTrue(downloadingButPackMissing.contains("speech pack", ignoreCase = true))
        assertFalse(downloadingButPackMissing.contains("Downloading", ignoreCase = true))

        val downloadingOk = LiveTranslationText.radioOverlayPrimary(
            pausedForPtt = false,
            ttsEnabled = false,
            modelDownloading = true,
            needsSpeechPack = false,
            statusError = false,
            translation = "",
            caption = "",
            listening = false,
            sourceCode = "en",
            targetCode = "es",
        )
        assertTrue(downloadingOk.contains("Downloading", ignoreCase = true))
    }

    @Test
    fun `setupHint guides first-run steps`() {
        val downloading = LiveTranslationText.setupHint(
            modelsReady = false,
            modelDownloading = true,
            modelFailed = false,
            speechOk = false,
            wifiOnly = true,
        )
        assertTrue(downloading.contains("speech pack", ignoreCase = true))
        assertFalse(downloading.contains("downloading", ignoreCase = true))

        val speech = LiveTranslationText.setupHint(
            modelsReady = true,
            modelDownloading = false,
            modelFailed = false,
            speechOk = false,
            wifiOnly = true,
        )
        assertTrue(speech.contains("speech pack", ignoreCase = true))
        assertTrue(speech.contains("Settings", ignoreCase = true))

        val ready = LiveTranslationText.setupHint(
            modelsReady = true,
            modelDownloading = false,
            modelFailed = false,
            speechOk = true,
            wifiOnly = true,
        )
        assertTrue(ready.contains("Ready"))
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
