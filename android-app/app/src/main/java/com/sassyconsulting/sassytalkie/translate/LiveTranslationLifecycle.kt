// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-LTL3P9W7KMQX
package com.sassyconsulting.sassytalkie.translate

/**
 * Pure mic-coordination state for live captioning vs PTT / UI consumers.
 * Kept free of Android APIs so JVM unit tests can cover start/stop decisions.
 */
class LiveTranslationLifecycle {

    /** What the bridge should do to the recognizer after a state transition. */
    enum class MicAction { START, STOP, NONE }

    var enabled: Boolean = false
        private set
    var pausedForPtt: Boolean = false
        private set
    var uiConsumers: Int = 0
        private set

    fun setEnabled(on: Boolean): MicAction {
        if (enabled == on) return MicAction.NONE
        enabled = on
        if (!on) {
            pausedForPtt = false
            // Always STOP on disable even if screen/process consumers remain —
            // leftover acquires must not keep the recognizer alive.
            return MicAction.STOP
        }
        return desiredAction(pttActive = false)
    }

    fun acquireUi(): MicAction {
        uiConsumers++
        return desiredAction(pttActive = false)
    }

    fun releaseUi(): MicAction {
        uiConsumers = (uiConsumers - 1).coerceAtLeast(0)
        return if (uiConsumers == 0) MicAction.STOP else MicAction.NONE
    }

    fun onPttStarted(): MicAction {
        if (!enabled) return MicAction.NONE
        // Idempotent — coordinator + native both call this on the same press.
        if (pausedForPtt) return MicAction.NONE
        pausedForPtt = true
        return MicAction.STOP
    }

    /**
     * Clear the PTT pause. Caller should invoke this after the post-PTT delay.
     * Returns START when captioning should reclaim the mic.
     */
    fun onPttResumeReady(pttStillActive: Boolean): MicAction {
        if (!enabled) {
            pausedForPtt = false
            return MicAction.NONE
        }
        if (pttStillActive) {
            pausedForPtt = true
            return MicAction.NONE
        }
        pausedForPtt = false
        return desiredAction(pttActive = false)
    }

    /** True when the recognizer should be listening right now. */
    fun shouldRun(pttActive: Boolean): Boolean =
        enabled && !pausedForPtt && uiConsumers > 0 && !pttActive

    private fun desiredAction(pttActive: Boolean): MicAction =
        if (shouldRun(pttActive)) MicAction.START else MicAction.NONE
}

/** Format a local caption/translation pair for the Timeline feed. */
object LiveTranslationText {
    fun timelineEntry(caption: String, translation: String): String? {
        val heard = caption.trim()
        if (heard.isEmpty()) return null
        val translated = translation.trim()
        return when {
            translated.isNotEmpty() &&
                !translated.equals(heard, ignoreCase = true) ->
                "$translated\n($heard)"
            else -> heard
        }
    }

    /**
     * Text to re-speak from a timeline row. Prefers the translated line
     * (`translated\n(heard)`), otherwise the whole caption.
     */
    fun speakableFromTimeline(text: String): String {
        val trimmed = text.trim()
        if (trimmed.isEmpty()) return ""
        val firstLine = trimmed.lineSequence().firstOrNull()?.trim().orEmpty()
        return firstLine.ifEmpty { trimmed }
    }

    /** Human label for a BCP-47 code from the common list, else the code. */
    fun languageLabel(code: String): String =
        TranslationManager.COMMON_LANGUAGES.firstOrNull { it.code == code }?.label ?: code

    /** Status copy while ML Kit models are downloading for a pair. */
    fun downloadStatusLine(sourceCode: String, targetCode: String): String {
        val src = languageLabel(sourceCode)
        val dst = languageLabel(targetCode)
        return "Downloading $src → $dst model (~30 MB)…"
    }

    /** Prefer translated line; fall back to caption. */
    fun speakableUtterance(caption: String, translation: String): String =
        translation.trim().ifEmpty { caption.trim() }

    /** Collapse whitespace for duplicate-final detection. */
    fun normalizeKey(text: String): String =
        text.trim().lowercase().replace(Regex("\\s+"), " ")

    /** TTS should only play when enabled and not fighting TX/RX audio. */
    fun shouldSpeakTts(
        ttsEnabled: Boolean,
        pausedForPtt: Boolean,
        incomingAudio: Boolean,
    ): Boolean = ttsEnabled && !pausedForPtt && !incomingAudio

    /**
     * When TTS is on but TX/RX owns the audio path, queue the line for
     * post-PTT / post-RX read-back instead of dropping it.
     */
    fun shouldQueueTts(
        ttsEnabled: Boolean,
        pausedForPtt: Boolean,
        incomingAudio: Boolean,
    ): Boolean = ttsEnabled && (pausedForPtt || incomingAudio)

    /** True when the offline speech pack error copy should prompt system Settings. */
    fun needsOfflineSpeechPack(errorMessage: String?): Boolean {
        val msg = errorMessage ?: return false
        return msg.contains("Offline language", ignoreCase = true) ||
            msg.contains("not installed", ignoreCase = true) ||
            msg.contains("Language not supported offline", ignoreCase = true) ||
            msg.contains("speech recognition unavailable", ignoreCase = true)
    }

    /**
     * First-run checklist copy for ML Kit models + system speech packs.
     * [modelsReady] = both pair languages on-device; [speechOk] = recognizer not UNAVAILABLE.
     */
    fun setupHint(
        modelsReady: Boolean,
        modelDownloading: Boolean,
        modelFailed: Boolean,
        speechOk: Boolean,
        wifiOnly: Boolean,
    ): String = when {
        modelDownloading -> "Step 1 of 2: downloading translation models (~30 MB)…"
        modelFailed && wifiOnly ->
            "Translation models need Wi-Fi (or turn off Wi-Fi-only downloads)."
        modelFailed -> "Translation model download failed — tap Retry."
        !modelsReady -> "Step 1 of 2: translation models will download for your language pair."
        !speechOk ->
            "Step 2 of 2: install an offline speech pack in system Settings (Voice / Languages)."
        else -> "Ready — speak while idle to caption; release PTT to hear translation read-back."
    }
}
