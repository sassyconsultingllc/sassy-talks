package com.sassyconsulting.sassytalkie.ui.util

import android.graphics.Bitmap
import android.graphics.Color
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel

/**
 * Efficient QR bitmap generation for on-screen display.
 *
 * Encodes at the QR module grid size, then scales once to the target pixel
 * size — avoids allocating oversized bitmaps and per-pixel [Bitmap.setPixel]
 * loops (Play Console "bitmap downsampling" recommendation).
 */
object QrBitmap {

    fun generate(content: String, displaySizePx: Int): Bitmap? {
        if (content.isEmpty() || displaySizePx <= 0) return null
        return try {
            val hints = mapOf(
                EncodeHintType.MARGIN to 1,
                EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.M,
            )
            val matrix = QRCodeWriter().encode(
                content,
                BarcodeFormat.QR_CODE,
                displaySizePx,
                displaySizePx,
                hints,
            )
            val w = matrix.width
            val h = matrix.height
            val pixels = IntArray(w * h)
            var i = 0
            for (y in 0 until h) {
                for (x in 0 until w) {
                    pixels[i++] = if (matrix[x, y]) Color.BLACK else Color.WHITE
                }
            }
            val raw = Bitmap.createBitmap(w, h, Bitmap.Config.RGB_565)
            raw.setPixels(pixels, 0, w, 0, 0, w, h)
            if (w == displaySizePx && h == displaySizePx) {
                raw
            } else {
                Bitmap.createScaledBitmap(raw, displaySizePx, displaySizePx, false).also {
                    if (it !== raw) raw.recycle()
                }
            }
        } catch (_: Exception) {
            null
        }
    }
}
