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

    /** Available Whisper model configurations */
    enum class ModelSize(val filename: String, val url: String, val expectedBytes: Long, val label: String) {
        BASE_EN(
            "ggml-base.en.bin",
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin",
            147_964_211L,
            "Base (142 MB)"
        ),
        SMALL_EN(
            "ggml-small.en.bin",
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin",
            487_601_967L,
            "Small (466 MB)"
        ),
        MEDIUM_EN(
            "ggml-medium.en.bin",
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.en.bin",
            1_533_774_781L,
            "Medium (1.5 GB)"
        ),
        LARGE_V3(
            "ggml-large-v3.bin",
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin",
            3_095_033_483L,
            "Large (3.1 GB)"
        );
    }

    /** Current model selection — persisted in SharedPreferences */
    @Volatile
    var currentModel: ModelSize = ModelSize.BASE_EN
        private set

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
        return File(dir, currentModel.filename).absolutePath
    }

    /** Check if model is already downloaded and valid. */
    fun isModelReady(context: Context): Boolean {
        val file = File(modelPath(context))
        return file.exists() && file.length() > 1_000_000 // basic sanity check
    }

    /** Load persisted model preference */
    private fun loadModelPreference(context: Context) {
        val prefs = context.getSharedPreferences("sassy_whisper", Context.MODE_PRIVATE)
        val saved = prefs.getString("model_size", "BASE_EN") ?: "BASE_EN"
        currentModel = try { ModelSize.valueOf(saved) } catch (_: Exception) { ModelSize.BASE_EN }
    }

    /** Switch to a different model size. Triggers re-download if needed. */
    fun switchModel(context: Context, model: ModelSize) {
        currentModel = model
        context.getSharedPreferences("sassy_whisper", Context.MODE_PRIVATE)
            .edit().putString("model_size", model.name).apply()
        _state.value = ModelState.NotDownloaded
        TranscriptionBridge.whisperReady = false
        ensureReady(context)
    }

    /**
     * Ensure model is available and whisper is initialized.
     * Downloads if needed, then calls TranscriptionBridge.initWhisper().
     */
    fun ensureReady(context: Context) {
        if (_state.value is ModelState.Ready) return

        loadModelPreference(context)
        val path = modelPath(context)
        if (isModelReady(context)) {
            // Model exists — just init whisper
            initWhisper(path)
            return
        }

        // Delete old model files to save space
        val dir = File(context.filesDir, "whisper")
        dir.listFiles()?.forEach { f ->
            if (f.name != currentModel.filename) {
                f.delete()
                Log.i(TAG, "Deleted old model: ${f.name}")
            }
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

        val model = currentModel
        val outFile = File(dir, model.filename)
        val tmpFile = File(dir, "${model.filename}.tmp")

        Log.i(TAG, "Downloading ${model.label} from ${model.url}")

        val conn = URL(model.url).openConnection() as HttpURLConnection
        conn.connectTimeout = 30_000
        conn.readTimeout = 60_000
        conn.instanceFollowRedirects = true

        try {
            conn.connect()
            val code = conn.responseCode
            if (code != 200) throw Exception("HTTP $code")

            val totalBytes = conn.contentLengthLong.let {
                if (it > 0) it else model.expectedBytes
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
