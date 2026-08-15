// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-UP4TDY7JEA3K
package com.sassyconsulting.sassytalkie.translate

import android.util.Log
import com.google.mlkit.common.model.DownloadConditions
import com.google.mlkit.common.model.RemoteModelManager
import com.google.mlkit.nl.translate.TranslateLanguage
import com.google.mlkit.nl.translate.TranslateRemoteModel
import com.google.mlkit.nl.translate.Translation
import com.google.mlkit.nl.translate.Translator
import com.google.mlkit.nl.translate.TranslatorOptions
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeout
import java.util.Collections
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/**
 * On-device, offline translation wrapper around ML Kit's Translate API.
 *
 * Everything here runs fully on-device once the per-language models are
 * downloaded. The first download for a given language pulls a ~30 MB model
 * over the network (controllable via [DownloadConditions] — Wi-Fi-only by
 * default); after that, translation never touches the network, consistent
 * with the app's privacy story.
 *
 * ── 16 KB page-size note ────────────────────────────────────────────────
 * The team previously hit Play's 16 KB ELF-alignment requirement with the
 * BUNDLED ML Kit barcode `.so` (libbarhopper_v3.so, frozen at 4 KB) and had
 * to switch to the play-services (unbundled) barcode variant. ML Kit
 * Translate (`com.google.mlkit:translate:17.0.3`) is a BUNDLED dependency —
 * it ships native libraries (TFLite / language-id) inside our APK. Before
 * release, the bundled translate `.so` files MUST be validated against the
 * 16 KB page-size requirement (e.g. `check_elf_alignment.sh` from the NDK,
 * or `objdump -p | grep LOAD`). If any segment is < 16 KB aligned, bump to a
 * newer translate version or coordinate with Google as was done for barcode.
 * There is currently no play-services (unbundled) variant of Translate.
 * ────────────────────────────────────────────────────────────────────────
 *
 * Threading: all suspend functions wrap ML Kit's `Task` callbacks and never
 * block. Translators are pooled per (src,dst) pair and reused; close them via
 * [release] when the owning screen/feature goes away.
 */
class TranslationManager {

    companion object {
        private const val TAG = "TranslationManager"

        /** Max number of recent (text → translation) results kept in the LRU cache. */
        private const val CACHE_CAPACITY = 128

        /** Fail closed to FAILED rather than spinning "Downloading…" forever. */
        const val MODEL_DOWNLOAD_TIMEOUT_MS = 45_000L

        /**
         * Convenience: the BCP-47 codes ML Kit accepts for a handful of common
         * targets, with human labels for a picker. ML Kit supports ~50+ languages;
         * this is a curated short-list for the UI. Codes come from
         * [TranslateLanguage]; passing an unsupported code is handled gracefully.
         */
        val COMMON_LANGUAGES: List<Language> = listOf(
            Language(TranslateLanguage.ENGLISH, "English"),
            Language(TranslateLanguage.SPANISH, "Spanish"),
            Language(TranslateLanguage.FRENCH, "French"),
            Language(TranslateLanguage.GERMAN, "German"),
            Language(TranslateLanguage.ITALIAN, "Italian"),
            Language(TranslateLanguage.PORTUGUESE, "Portuguese"),
            Language(TranslateLanguage.DUTCH, "Dutch"),
            Language(TranslateLanguage.RUSSIAN, "Russian"),
            Language(TranslateLanguage.CHINESE, "Chinese"),
            Language(TranslateLanguage.JAPANESE, "Japanese"),
            Language(TranslateLanguage.KOREAN, "Korean"),
            Language(TranslateLanguage.ARABIC, "Arabic"),
            Language(TranslateLanguage.HINDI, "Hindi"),
            Language(TranslateLanguage.VIETNAMESE, "Vietnamese"),
            Language(TranslateLanguage.UKRAINIAN, "Ukrainian"),
            Language(TranslateLanguage.POLISH, "Polish"),
            Language(TranslateLanguage.TURKISH, "Turkish"),
        )

        /**
         * Validate + normalize a language tag (e.g. "en", "es") to the exact
         * code ML Kit recognizes. Returns null if ML Kit can't translate it.
         */
        fun normalizeLanguage(tag: String): String? =
            TranslateLanguage.fromLanguageTag(tag)
    }

    /** A selectable language: ML Kit BCP-47 code + a human label for the UI. */
    data class Language(val code: String, val label: String)

    /** Progress/availability state for a single (src→dst) model pair. */
    enum class ModelState { UNKNOWN, NOT_DOWNLOADED, DOWNLOADING, READY, FAILED }

    private val modelManager = RemoteModelManager.getInstance()

    private val translatorLock = Any()
    private val translators = HashMap<String, Translator>()

    // ── LRU cache of recent translations ────────────────────────────────
    // Keyed by "src|dst|text". accessOrder=true makes this a true LRU; we
    // evict the eldest entry once over capacity. Synchronized for cross-thread
    // access (STT callbacks vs. UI).
    private val cache = Collections.synchronizedMap(
        object : LinkedHashMap<String, String>(16, 0.75f, true) {
            override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, String>?): Boolean =
                size > CACHE_CAPACITY
        }
    )

    // ── Per-pair model state for the UI to observe ──────────────────────
    private val _modelState = MutableStateFlow(ModelState.UNKNOWN)
    /** Coarse state of the MOST RECENTLY requested model pair (drives the UI badge). */
    val modelState: StateFlow<ModelState> = _modelState.asStateFlow()

    private fun pairKey(src: String, dst: String) = "$src|$dst"
    private fun cacheKey(src: String, dst: String, text: String) = "$src|$dst|$text"

    /**
     * Get (or lazily build) a pooled [Translator] for the given language pair.
     * Building a Translator does NOT download anything — that happens on first
     * `translate`/`downloadModelsIfNeeded`.
     */
    private fun translatorFor(src: String, dst: String): Translator {
        val key = pairKey(src, dst)
        synchronized(translatorLock) {
            translators[key]?.let { return it }
            val options = TranslatorOptions.Builder()
                .setSourceLanguage(src)
                .setTargetLanguage(dst)
                .build()
            val created = Translation.getClient(options)
            translators[key] = created
            return created
        }
    }

    /**
     * Ensure both the source and target models for this pair are present on
     * device, downloading them if needed. Updates [modelState] as it goes.
     *
     * @param requireWifi when true (default), models only download over an
     *   unmetered (Wi-Fi) connection — avoids surprise cellular data usage.
     * @return true if the models are ready after this call; false on failure.
     */
    suspend fun downloadModelsIfNeeded(src: String, dst: String, requireWifi: Boolean = true): Boolean {
        // Fast path: both models already on device — don't flash DOWNLOADING.
        val onDevice = downloadedModels().toSet()
        if (src in onDevice && dst in onDevice) {
            _modelState.value = ModelState.READY
            return true
        }

        val translator = translatorFor(src, dst)
        val conditions = DownloadConditions.Builder().apply {
            if (requireWifi) requireWifi()
        }.build()

        _modelState.value = ModelState.DOWNLOADING
        return try {
            withTimeout(MODEL_DOWNLOAD_TIMEOUT_MS) {
                awaitTask<Void>("downloadModelIfNeeded") { translator.downloadModelIfNeeded(conditions) }
            }
            _modelState.value = ModelState.READY
            true
        } catch (e: TimeoutCancellationException) {
            Log.w(TAG, "Model download timed out for $src→$dst")
            _modelState.value = ModelState.FAILED
            false
        } catch (e: CancellationException) {
            // Language switch cancelled this job — don't flash FAILED.
            throw e
        } catch (t: Throwable) {
            Log.w(TAG, "Model download failed for $src→$dst: ${t.message}")
            _modelState.value = ModelState.FAILED
            false
        }
    }

    /**
     * Translate [text] from [src] to [dst], offline. Returns the original text
     * unchanged if src == dst, the text is blank, or translation fails (never
     * throws — captioning should degrade to showing the source text).
     *
     * Triggers a model download if needed (honoring [requireWifi]); callers
     * that want to gate on Wi-Fi should call [downloadModelsIfNeeded] first and
     * observe [modelState].
     */
    suspend fun translate(
        text: String,
        src: String,
        dst: String,
        requireWifi: Boolean = true,
    ): String {
        val trimmed = text.trim()
        if (trimmed.isEmpty()) return text
        if (src == dst) return text

        cache[cacheKey(src, dst, trimmed)]?.let { return it }

        val translator = translatorFor(src, dst)
        // Avoid flashing "Downloading…" on every final once models are READY.
        val announceDownload = _modelState.value != ModelState.READY
        return try {
            if (announceDownload && _modelState.value != ModelState.DOWNLOADING) {
                _modelState.value = ModelState.DOWNLOADING
            }
            val conditions = DownloadConditions.Builder().apply {
                if (requireWifi) requireWifi()
            }.build()
            withTimeout(MODEL_DOWNLOAD_TIMEOUT_MS) {
                awaitTask<Void>("downloadModelIfNeeded") { translator.downloadModelIfNeeded(conditions) }
            }
            _modelState.value = ModelState.READY

            val result = awaitTask<String>("translate") { translator.translate(trimmed) }
            cache[cacheKey(src, dst, trimmed)] = result
            result
        } catch (e: TimeoutCancellationException) {
            Log.w(TAG, "translate($src→$dst) model download timed out")
            if (announceDownload) _modelState.value = ModelState.FAILED
            text
        } catch (e: CancellationException) {
            throw e
        } catch (t: Throwable) {
            Log.w(TAG, "translate($src→$dst) failed: ${t.message}")
            if (announceDownload) _modelState.value = ModelState.FAILED
            // Graceful fallback: surface the original so the UI isn't blank.
            text
        }
    }

    /**
     * List the language codes (BCP-47) whose translation model is currently
     * downloaded on this device. Empty list on error.
     */
    suspend fun downloadedModels(): List<String> = try {
        val models = awaitTask<Set<TranslateRemoteModel>>("getDownloadedModels") {
            modelManager.getDownloadedModels(TranslateRemoteModel::class.java)
        }
        models.map { it.language }
    } catch (t: Throwable) {
        Log.w(TAG, "getDownloadedModels failed: ${t.message}")
        emptyList()
    }

    /** True if the model for a single language [code] is already downloaded. */
    suspend fun isModelDownloaded(code: String): Boolean =
        downloadedModels().contains(code)

    /**
     * Delete a single downloaded language model to reclaim storage. No-op if
     * it isn't present. Returns true on success.
     */
    suspend fun deleteModel(code: String): Boolean = try {
        val model = TranslateRemoteModel.Builder(code).build()
        awaitTask<Void>("deleteDownloadedModel") {
            modelManager.deleteDownloadedModel(model)
        }
        true
    } catch (t: Throwable) {
        Log.w(TAG, "deleteModel($code) failed: ${t.message}")
        false
    }

    /** Drop the in-memory translation cache (e.g. on target-language change). */
    fun clearCache() = cache.clear()

    /**
     * Close all pooled translators and clear the cache. Call when the feature
     * is turned off or the owning scope is destroyed. Idempotent.
     */
    fun release() {
        synchronized(translatorLock) {
            translators.values.forEach { runCatching { it.close() } }
            translators.clear()
        }
        cache.clear()
        _modelState.value = ModelState.UNKNOWN
    }

    /**
     * Bridge an ML Kit [com.google.android.gms.tasks.Task] to a suspend
     * function. Cancellable and non-blocking; resumes on the Task's callback.
     */
    private suspend fun <T> awaitTask(
        op: String,
        block: () -> com.google.android.gms.tasks.Task<T>,
    ): T = suspendCancellableCoroutine { cont ->
        try {
            block()
                .addOnSuccessListener { result -> if (cont.isActive) cont.resume(result) }
                .addOnFailureListener { e -> if (cont.isActive) cont.resumeWithException(e) }
                .addOnCanceledListener { if (cont.isActive) cont.cancel() }
        } catch (t: Throwable) {
            // ML Kit/Play Services unavailable, etc. — fail the coroutine
            // rather than crashing the caller.
            Log.w(TAG, "$op threw synchronously: ${t.message}")
            if (cont.isActive) cont.resumeWithException(t)
        }
    }
}
