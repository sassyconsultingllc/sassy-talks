package com.sassyconsulting.sassytalkie.ui

import androidx.compose.animation.core.*
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.cos
import kotlin.math.sin

/**
 * Rainbow arc refresh spinner.
 *
 * 30° bright active arc with full rainbow gradient,
 * 330° faded remainder that trails behind.
 * Smooth continuous rotation at ~1.2s per revolution.
 *
 * Tap to trigger [onRefresh]. Spins while [isRefreshing] is true,
 * otherwise shows a static rainbow ring at low opacity as an idle hint.
 */
@Composable
fun RainbowRefreshIndicator(
    isRefreshing: Boolean,
    onRefresh: () -> Unit,
    modifier: Modifier = Modifier,
    size: Dp = 28.dp,
    strokeWidth: Dp = 3.dp
) {
    // Smooth infinite rotation: 0 → 360 over 1200ms
    val infiniteTransition = rememberInfiniteTransition(label = "rainbowSpin")
    val rotation by infiniteTransition.animateFloat(
        initialValue = 0f,
        targetValue = 360f,
        animationSpec = infiniteRepeatable(
            animation = tween(durationMillis = 1200, easing = LinearEasing),
            repeatMode = RepeatMode.Restart
        ),
        label = "rotation"
    )

    // Rainbow hue palette — 7 stops across the 30° active arc
    val rainbowColors = listOf(
        Color(0xFFFF0000), // Red
        Color(0xFFFF8800), // Orange
        Color(0xFFFFFF00), // Yellow
        Color(0xFF00FF00), // Green
        Color(0xFF00CCFF), // Cyan
        Color(0xFF0044FF), // Blue
        Color(0xFFAA00FF)  // Violet
    )

    val currentRotation = if (isRefreshing) rotation else 0f

    Canvas(
        modifier = modifier
            .size(size)
            .clickable { onRefresh() }
    ) {
        val canvasSize = this.size.minDimension
        val stroke = strokeWidth.toPx()
        val radius = (canvasSize - stroke) / 2f
        val center = Offset(canvasSize / 2f, canvasSize / 2f)
        val arcRect = Size(radius * 2f, radius * 2f)
        val topLeft = Offset(center.x - radius, center.y - radius)

        val activeSweep = 30f  // 30° bright arc
        val fadedSweep = 330f  // 330° faded remainder

        if (isRefreshing) {
            // --- Faded 330° remainder ---
            // Draw it behind the active arc, starting right after the active arc ends
            val fadedStart = currentRotation + activeSweep
            // Split faded arc into small segments for subtle rainbow tint
            val fadedSegments = 33
            val segSweep = fadedSweep / fadedSegments
            for (i in 0 until fadedSegments) {
                val segAngle = fadedStart + i * segSweep
                // Map segment position to rainbow hue
                val hueT = (i.toFloat() / fadedSegments)
                val colorIdx = (hueT * (rainbowColors.size - 1)).toInt()
                    .coerceIn(0, rainbowColors.size - 2)
                val colorFrac = (hueT * (rainbowColors.size - 1)) - colorIdx
                val segColor = lerpColor(
                    rainbowColors[colorIdx],
                    rainbowColors[(colorIdx + 1).coerceAtMost(rainbowColors.size - 1)],
                    colorFrac
                )
                // Fade: alpha ramps down from 0.18 near active arc to 0.04 at the tail
                val fadeAlpha = 0.18f - (0.14f * (i.toFloat() / fadedSegments))
                drawArc(
                    color = segColor.copy(alpha = fadeAlpha),
                    startAngle = segAngle - 90f, // Canvas 0° is at 3 o'clock, offset to 12
                    sweepAngle = segSweep + 0.5f, // Tiny overlap to prevent gaps
                    useCenter = false,
                    topLeft = topLeft,
                    size = arcRect,
                    style = Stroke(width = stroke, cap = StrokeCap.Butt)
                )
            }

            // --- Bright 30° active arc ---
            // Split into small segments for rainbow gradient effect
            val activeSegments = 15
            val activeSegSweep = activeSweep / activeSegments
            for (i in 0 until activeSegments) {
                val segAngle = currentRotation + i * activeSegSweep
                val hueT = i.toFloat() / (activeSegments - 1).coerceAtLeast(1)
                val colorIdx = (hueT * (rainbowColors.size - 1)).toInt()
                    .coerceIn(0, rainbowColors.size - 2)
                val colorFrac = (hueT * (rainbowColors.size - 1)) - colorIdx
                val segColor = lerpColor(
                    rainbowColors[colorIdx],
                    rainbowColors[(colorIdx + 1).coerceAtMost(rainbowColors.size - 1)],
                    colorFrac
                )
                val cap = if (i == 0) StrokeCap.Round
                          else if (i == activeSegments - 1) StrokeCap.Round
                          else StrokeCap.Butt
                drawArc(
                    color = segColor,
                    startAngle = segAngle - 90f,
                    sweepAngle = activeSegSweep + 0.5f,
                    useCenter = false,
                    topLeft = topLeft,
                    size = arcRect,
                    style = Stroke(width = stroke, cap = cap)
                )
            }
        } else {
            // --- Idle state: static rainbow ring at low alpha ---
            val segments = 36
            val segSweep = 360f / segments
            for (i in 0 until segments) {
                val hueT = i.toFloat() / segments
                val colorIdx = (hueT * (rainbowColors.size - 1)).toInt()
                    .coerceIn(0, rainbowColors.size - 2)
                val colorFrac = (hueT * (rainbowColors.size - 1)) - colorIdx
                val segColor = lerpColor(
                    rainbowColors[colorIdx],
                    rainbowColors[(colorIdx + 1).coerceAtMost(rainbowColors.size - 1)],
                    colorFrac
                )
                drawArc(
                    color = segColor.copy(alpha = 0.35f),
                    startAngle = (i * segSweep) - 90f,
                    sweepAngle = segSweep + 0.5f,
                    useCenter = false,
                    topLeft = topLeft,
                    size = arcRect,
                    style = Stroke(width = stroke, cap = StrokeCap.Butt)
                )
            }

            // Draw a small refresh arrow hint in center
            val arrowRadius = radius * 0.45f
            val arrowColor = Color.White.copy(alpha = 0.5f)
            // Draw a simple curved arrow using arc + head
            drawArc(
                color = arrowColor,
                startAngle = -60f,
                sweepAngle = 240f,
                useCenter = false,
                topLeft = Offset(center.x - arrowRadius, center.y - arrowRadius),
                size = Size(arrowRadius * 2f, arrowRadius * 2f),
                style = Stroke(width = stroke * 0.8f, cap = StrokeCap.Round)
            )
            // Arrow head at the end of the arc (at ~180° = left side)
            val headAngle = Math.toRadians(180.0)
            val headX = center.x + arrowRadius * cos(headAngle).toFloat()
            val headY = center.y + arrowRadius * sin(headAngle).toFloat()
            val headLen = stroke * 2.5f
            // Two lines forming arrow head pointing clockwise
            drawLine(
                color = arrowColor,
                start = Offset(headX, headY),
                end = Offset(headX + headLen * 0.5f, headY - headLen * 0.7f),
                strokeWidth = stroke * 0.8f,
                cap = StrokeCap.Round
            )
            drawLine(
                color = arrowColor,
                start = Offset(headX, headY),
                end = Offset(headX - headLen * 0.5f, headY - headLen * 0.5f),
                strokeWidth = stroke * 0.8f,
                cap = StrokeCap.Round
            )
        }
    }
}

/** Linearly interpolate between two colors */
private fun lerpColor(c1: Color, c2: Color, t: Float): Color {
    val clampedT = t.coerceIn(0f, 1f)
    return Color(
        red = c1.red + (c2.red - c1.red) * clampedT,
        green = c1.green + (c2.green - c1.green) * clampedT,
        blue = c1.blue + (c2.blue - c1.blue) * clampedT,
        alpha = c1.alpha + (c2.alpha - c1.alpha) * clampedT
    )
}
