//  Theme.swift
//  SassyTalkie — shared design tokens
//
//  Ported 1:1 from the Tauri desktop reference
//  (tauri-desktop/src/styles/app.css :root) so iOS matches the cross-platform
//  look: slate-dark surfaces with blue → purple → teal → coral accents.
//  Keep in sync with app.css and android .../ui/theme/Color.kt.
//
//  Note: `Color(hex:)` is declared in ContentView.swift (same module) and
//  reused here — do not redeclare it.

import SwiftUI

extension Color {
    // Brand / accent ramps
    static let stPrimaryBlue      = Color(hex: "2563EB")
    static let stPrimaryBlueLight = Color(hex: "3B82F6")
    static let stPrimaryBlueDark  = Color(hex: "1D4ED8")

    static let stPurple           = Color(hex: "7C3AED")
    static let stPurpleLight      = Color(hex: "8B5CF6")
    static let stPurpleDark       = Color(hex: "6D28D9")

    static let stTeal             = Color(hex: "14B8A6")
    static let stTealLight        = Color(hex: "2DD4BF")
    static let stTealDark         = Color(hex: "0D9488")

    static let stCoral            = Color(hex: "F97316")
    static let stCoralLight       = Color(hex: "FB923C")
    static let stCoralDark        = Color(hex: "EA580C")

    // Surfaces
    static let stBgDark           = Color(hex: "0F172A")
    static let stBgMedium         = Color(hex: "1E293B")
    static let stBgLight          = Color(hex: "334155")
    static let stBgCard           = Color(hex: "1E293B").opacity(0.8)
    static let stBgStatusBar      = Color(hex: "0B1120")

    // Text
    static let stTextPrimary      = Color(hex: "F8FAFC")
    static let stTextSecondary    = Color(hex: "94A3B8")
    static let stTextMuted        = Color(hex: "64748B")

    // Status
    static let stOnline           = Color(hex: "22C55E")
    static let stWarning          = Color(hex: "EAB308")
    static let stError            = Color(hex: "EF4444")
    static let stInfo             = Color.stPrimaryBlue

    // Lines
    static let stBorder           = Color(hex: "94A3B8").opacity(0.2)
}

/// Design-system helpers (gradients, fonts, radii, spacing) mirroring app.css.
enum SassyTheme {
    // CSS 135deg linear-gradient == topLeading -> bottomTrailing.
    static func gradient(_ stops: [Color]) -> LinearGradient {
        LinearGradient(colors: stops, startPoint: .topLeading, endPoint: .bottomTrailing)
    }
    static let gradientPrimary = gradient([.stPrimaryBlue, .stPurple]) // brand title / primary fill
    static let gradientAccent  = gradient([.stTeal, .stPrimaryBlue])   // "Find Devices" CTA
    static let gradientWarm    = gradient([.stCoral, .stPurple])
    static let gradientCool    = gradient([.stTeal, .stPurple])

    // Typography — Inter ≈ system (SF), JetBrains Mono ≈ SF Mono.
    static func ui(_ size: CGFloat, _ weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight)
    }
    static func mono(_ size: CGFloat, _ weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight, design: .monospaced)
    }

    // Radii / spacing (px → pt 1:1, matching app.css tokens)
    static let radiusSm: CGFloat = 8
    static let radiusMd: CGFloat = 12
    static let radiusLg: CGFloat = 20

    static let spaceXS: CGFloat = 4
    static let spaceSM: CGFloat = 8
    static let spaceMD: CGFloat = 16
    static let spaceLG: CGFloat = 24
    static let spaceXL: CGFloat = 32
}
