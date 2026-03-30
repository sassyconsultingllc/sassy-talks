package com.sassyconsulting.sassytalkie

import android.content.Context
import android.util.Log
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * Manages downloading and initializing the Whisper tiny model for on-device
 * speech-to-text. The model (~75 MB) is downloaded once and cached in the
 * app's internal files directory.
 */
object WhisperModelManager {

    private const val TAG = "WhisperModel"

    /** Model file name stored in app's filesDir/whisper/ */
    private const val MODEL_FILENAME = "ggml-tiny.en.bin"

    /** CDN URL for the model. Hosted on Hugging Face (public, fast). */
    private const val MODEL_URL =
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin"

    /** Model size in bytes (~75 MB) for progress calculation. */
    private const val EXPECTED_SIZE_BYTES = 77_704_448L

    // ── State ──

    sealed class ModelState {
        data object NotDownloaded : ModelState()
        data class Downloading(val progress: Float) : ModelState()  // 0.0 - 1.0
        data object Ready : ModelState()
        data class Error(val message: String) : ModelState()
    }

    private val _state = MutableStateFlow<ModelState>(ModelState.NotDownloaded)
    val state: StateFlow<ModelState> = _state.asStateFlow()

    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())

    /** Get the model file path (whether it exists or not). */
    fun modelPath(context: Context): String {
        val dir = File(context.filesDir, "whisper")
        return File(dir, MODEL_FILENAME).absolutePath
    }

    /** Check if model is already downloaded and valid. */
    fun isModelReady(context: Context): Boolean {
        val file = File(modelPath(context))
        return file.exists() && file.length() > 1_000_000 // basic sanity check
    }

    /**
     * Ensure model is available and whisper is initialized.
     * Downloads if needed, then calls TranscriptionBridge.initWhisper().
     */
    fun ensureReady(context: Context) {
        if (_state.value is ModelState.Ready) return

        val path = modelPath(context)
        if (isModelReady(context)) {
            // Model exists — just init whisper
            initWhisper(path)
            return
        }

        // Need to download
        _state.value = ModelState.Downloading(0f)
        scope.launch {
            try {
                downloadModel(context)
                initWhisper(path)
            } catch (e: Exception) {
                Log.e(TAG, "Model download failed: ${e.message}", e)
                _state.value = ModelState.Error(e.message ?: "Download failed")
            }
        }
    }

    private fun initWhisper(path: String) {
        val ok = TranscriptionBridge.initWhisper(path)
        _state.value = if (ok) ModelState.Ready else ModelState.Error("Failed to load model")
    }

    private suspend fun downloadModel(context: Context) = withContext(Dispatchers.IO) {
        val dir = File(context.filesDir, "whisper")
        if (!dir.exists()) dir.mkdirs()

        val outFile = File(dir, MODEL_FILENAME)
        val tmpFile = File(dir, "$MODEL_FILENAME.tmp")

        Log.i(TAG, "Downloading model from $MODEL_URL")

        val conn = URL(MODEL_URL).openConnection() as HttpURLConnection
        conn.connectTimeout = 30_000
        conn.readTimeout = 60_000
        conn.instanceFollowRedirects = true

        try {
            conn.connect()
            val code = conn.responseCode
            if (code != 200) throw Exception("HTTP $code")

            val totalBytes = conn.contentLengthLong.let {
                if (it > 0) it else EXPECTED_SIZE_BYTES
            }

            conn.inputStream.buffered().use { input ->
                tmpFile.outputStream().buffered().use { output ->
                    val buffer = ByteArray(65536)
                    var downloaded = 0L

                    while (true) {
                        val n = input.read(buffer)
                        if (n == -1) break
                        output.write(buffer, 0, n)
                        downloaded += n

                        val progress = (downloaded.toFloat() / totalBytes).coerceIn(0f, 1f)
                        _state.value = ModelState.Downloading(progress)
                    }
                }
            }

            // Atomic rename
            if (outFile.exists()) outFile.delete()
            tmpFile.renameTo(outFile)
            Log.i(TAG, "Model downloaded: ${outFile.length()} bytes")

        } finally {
            conn.disconnect()
            if (tmpFile.exists()) tmpFile.delete()
        }
    }
}
