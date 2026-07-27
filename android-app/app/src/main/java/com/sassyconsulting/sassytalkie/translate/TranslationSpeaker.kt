// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-TTS4N8K2RMQX
package com.sassyconsulting.sassytalkie.translate

import android.content.Context
import android.speech.tts.TextToSpeech
import android.util.Log
import java.util.Locale

/**
 * On-device TTS for reading back translated finals. Callers stop playback on
 * PTT and incoming peer audio so read-back never fights TX/RX.
 */
class TranslationSpeaker(context: Context) : TextToSpeech.OnInitListener {

    companion object {
        private const val TAG = "TranslationSpeaker"
        private const val UTTERANCE_PREFIX = "st_translate_"
    }

    private val appContext = context.applicationContext
    private var tts: TextToSpeech? = null
    @Volatile private var ready = false
    @Volatile private var langTag = TranslationLangDefaults.DEFAULT_TARGET
    private var utteranceSeq = 0

    init {
        try {
            tts = TextToSpeech(appContext, this)
        } catch (t: Throwable) {
            Log.w(TAG, "TTS unavailable: ${t.message}")
        }
    }

    override fun onInit(status: Int) {
        ready = status == TextToSpeech.SUCCESS
        if (ready) applyLanguage(langTag)
        else Log.w(TAG, "TTS init failed status=$status")
    }

    fun setLanguage(code: String) {
        langTag = code
        if (ready) applyLanguage(code)
    }

    /** Speak [text] if TTS is ready. No-ops when blank or not initialized. */
    fun speak(text: String) {
        val engine = tts ?: return
        val trimmed = text.trim()
        if (!ready || trimmed.isEmpty()) return
        try {
            utteranceSeq++
            engine.speak(
                trimmed,
                TextToSpeech.QUEUE_FLUSH,
                null,
                "$UTTERANCE_PREFIX$utteranceSeq",
            )
        } catch (t: Throwable) {
            Log.w(TAG, "speak failed: ${t.message}")
        }
    }

    fun stop() {
        try { tts?.stop() } catch (_: Throwable) {}
    }

    fun release() {
        ready = false
        try {
            tts?.stop()
            tts?.shutdown()
        } catch (_: Throwable) {}
        tts = null
    }

    private fun applyLanguage(code: String) {
        val engine = tts ?: return
        try {
            val locale = Locale.forLanguageTag(code)
            val result = engine.setLanguage(locale)
            if (result == TextToSpeech.LANG_MISSING_DATA ||
                result == TextToSpeech.LANG_NOT_SUPPORTED
            ) {
                Log.w(TAG, "TTS language unsupported: $code — falling back to default")
                engine.setLanguage(Locale.getDefault())
            }
        } catch (t: Throwable) {
            Log.w(TAG, "setLanguage($code) failed: ${t.message}")
        }
    }
}
