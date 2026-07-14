//
//  HostQRView.swift
//  SassyTalkie — renders a hosted channel's pairing QR for a joiner to scan.
//  Copyright © 2025 Sassy Consulting LLC. All rights reserved.
//
//  The displayed QR encodes the session JSON minted by
//  `sassytalkie_generate_session_qr`. Any SassyTalkie endpoint (iOS, Android,
//  desktop) can scan it to land on the same channel + key — the pairing is
//  cross-platform by construction since both sides use the shared core
//  `SessionManager`.

import SwiftUI
import UIKit
import CoreImage.CIFilterBuiltins

struct HostQRView: View {
    let json: String
    let channel: UInt8

    @Environment(\.presentationMode) private var presentationMode

    var body: some View {
        VStack(spacing: 18) {
            Text("Channel \(String(format: "%02d", channel))")
                .font(.title2).bold()
                .foregroundColor(.stTextPrimary)

            Text("Have the other device scan this to join")
                .font(.subheadline)
                .foregroundColor(.stTextSecondary)
                .multilineTextAlignment(.center)

            if let img = Self.qrImage(from: json) {
                Image(uiImage: img)
                    .interpolation(.none)   // keep QR modules crisp
                    .resizable()
                    .scaledToFit()
                    .frame(width: 260, height: 260)
                    .padding(12)
                    .background(Color.white)
                    .cornerRadius(12)
            } else {
                Text("Failed to render QR")
                    .foregroundColor(.stCoral)
            }

            Text("Encrypted · works with Android & desktop")
                .font(.caption)
                .foregroundColor(.stOnline)

            Button(action: { presentationMode.wrappedValue.dismiss() }) {
                Text("Done")
                    .font(.headline)
                    .foregroundColor(.stTeal)
            }
            .padding(.top, 4)
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.stBgDark.ignoresSafeArea())
    }

    /// Render a UTF-8 string into a QR `UIImage` via CoreImage, scaled up with
    /// nearest-neighbour so the modules stay sharp.
    private static func qrImage(from string: String) -> UIImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(string.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage else { return nil }
        let scaled = output.transformed(by: CGAffineTransform(scaleX: 10, y: 10))
        let context = CIContext()
        guard let cg = context.createCGImage(scaled, from: scaled.extent) else { return nil }
        return UIImage(cgImage: cg)
    }
}
