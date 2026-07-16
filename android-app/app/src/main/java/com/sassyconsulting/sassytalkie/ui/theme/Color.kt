// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-FC22G73VQWCY
package com.sassyconsulting.sassytalkie.ui.theme

import androidx.compose.ui.graphics.Color

// ─────────────────────────────────────────────────────────────────────────────
// SassyTalk shared design tokens — ported 1:1 from the Tauri desktop reference
// (tauri-desktop/src/styles/app.css :root). Single source of truth for the
// cross-platform look: slate-dark surfaces with blue → purple → teal → coral
// accents. Keep in sync with app.css and ios-native/Theme.swift.
// ─────────────────────────────────────────────────────────────────────────────

// Brand / accent ramps
val PrimaryBlue       = Color(0xFF2563EB)
val PrimaryBlueLight  = Color(0xFF3B82F6)
val PrimaryBlueDark   = Color(0xFF1D4ED8)

val BrandPurple       = Color(0xFF7C3AED)
val BrandPurpleLight  = Color(0xFF8B5CF6)
val BrandPurpleDark   = Color(0xFF6D28D9)

val Teal              = Color(0xFF14B8A6)
val TealLight         = Color(0xFF2DD4BF)
val TealDark          = Color(0xFF0D9488)

val Coral             = Color(0xFFF97316)
val CoralLight        = Color(0xFFFB923C)
val CoralDark         = Color(0xFFEA580C)

// Surfaces / backgrounds
val BgDark            = Color(0xFF0F172A)
val BgMedium          = Color(0xFF1E293B)
val BgLight           = Color(0xFF334155)
val BgCard            = Color(0xCC1E293B)   // rgba(30,41,59,0.8)
val BgStatusBar       = Color(0xFF0B1120)   // one notch darker than BgDark

// Text
val TextPrimary       = Color(0xFFF8FAFC)
val TextSecondary     = Color(0xFF94A3B8)
val TextMutedToken    = Color(0xFF64748B)

// Status
val StatusOnline      = Color(0xFF22C55E)
val StatusWarning     = Color(0xFFEAB308)
val StatusErrorToken  = Color(0xFFEF4444)
val StatusInfo        = PrimaryBlue

// Lines
val BorderColor       = Color(0x3394A3B8)   // rgba(148,163,184,0.2)

// Gradient stops — use with Brush.linearGradient / Brush.horizontalGradient.
// Mirror --gradient-primary / accent / warm / cool from app.css.
val GradientPrimary = listOf(PrimaryBlue, BrandPurple)   // brand title, primary fills
val GradientAccent  = listOf(Teal, PrimaryBlue)          // "Find Devices" CTA
val GradientWarm    = listOf(Coral, BrandPurple)
val GradientCool    = listOf(Teal, BrandPurple)

// ─────────────────────────────────────────────────────────────────────────────
// Backward-compatible aliases. The previous retro-terminal palette names are
// retargeted to the shared tokens so existing screens shift to the Tauri look
// without a rename churn. Prefer the token names above in new code.
// ─────────────────────────────────────────────────────────────────────────────
val Orange            = Coral
val OrangeDark        = CoralDark
val OrangeLight       = CoralLight

val Cyan              = Teal
val CyanDark          = TealDark
val Purple            = BrandPurple
val Green             = StatusOnline
val GreenDark         = Color(0xFF16A34A)

val DarkBg            = BgDark
val DarkerBg          = BgStatusBar
val CardBg            = BgMedium
val SurfaceBg         = BgLight

val TextWhite         = TextPrimary
val TextGray          = TextSecondary
val TextMuted         = TextMutedToken

val StatusConnected   = StatusOnline
val StatusDisconnected = StatusErrorToken
val StatusTransmitting = Coral
