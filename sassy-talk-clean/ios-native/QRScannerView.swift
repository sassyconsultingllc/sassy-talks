//
//  QRScannerView.swift
//  SassyTalkie — camera QR scanner for cross-platform pairing.
//  Copyright © 2025 Sassy Consulting LLC. All rights reserved.
//
//  Scans the host's pairing QR (the session JSON an Android/desktop/iOS host
//  generates) and hands the decoded string to `onScan`, which forwards it to
//  `sassytalkie_import_session_qr`. After a successful import the audio path is
//  AES-256-GCM encrypted (mandatory) and interops with Android on that channel.
//
//  REQUIRES `NSCameraUsageDescription` in Info.plist. Without it iOS terminates
//  the app via TCC (`__TCC_CRASHING_DUE_TO_PRIVACY_VIOLATION__` / abort_with_payload)
//  the instant the capture session starts — the same class of crash that took
//  down the mic path.

import SwiftUI
import AVFoundation
import UIKit

struct QRScannerView: View {
    /// Called with the decoded QR string on the main thread; the sheet dismisses
    /// itself afterward.
    var onScan: (String) -> Void

    @Environment(\.presentationMode) private var presentationMode

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()

            ScannerRepresentable(onScan: handle)
                .ignoresSafeArea()

            // Viewfinder + instructions overlay.
            RoundedRectangle(cornerRadius: 16)
                .stroke(Color.white.opacity(0.85), lineWidth: 3)
                .frame(width: 240, height: 240)

            VStack {
                HStack {
                    Spacer()
                    Button(action: { presentationMode.wrappedValue.dismiss() }) {
                        Image(systemName: "xmark.circle.fill")
                            .font(.system(size: 30))
                            .foregroundColor(.white.opacity(0.9))
                            .padding()
                    }
                }
                Spacer()
                Text("Point at the host's pairing QR")
                    .font(.headline)
                    .foregroundColor(.white)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                    .background(Color.black.opacity(0.55))
                    .cornerRadius(10)
                    .padding(.bottom, 44)
            }
        }
    }

    private func handle(_ code: String) {
        onScan(code)
        presentationMode.wrappedValue.dismiss()
    }
}

/// Bridges the UIKit AVFoundation capture stack into SwiftUI.
private struct ScannerRepresentable: UIViewControllerRepresentable {
    var onScan: (String) -> Void

    func makeUIViewController(context: Context) -> ScannerViewController {
        let vc = ScannerViewController()
        vc.onScan = onScan
        return vc
    }

    func updateUIViewController(_ vc: ScannerViewController, context: Context) {}
}

final class ScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    var onScan: ((String) -> Void)?

    private let session = AVCaptureSession()
    private var preview: AVCaptureVideoPreviewLayer?
    private var didScan = false

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black

        // Trigger the permission prompt explicitly so a first-run user gets the
        // system dialog rather than a silent black frame. NSCameraUsageDescription
        // MUST be present or the prompt path itself aborts the process via TCC.
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            configureSession()
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
                guard granted else { return }
                DispatchQueue.main.async {
                    self?.configureSession()
                    self?.startIfNeeded()
                }
            }
        default:
            break // denied/restricted — leave the black frame; user can close.
        }
    }

    private func configureSession() {
        guard let device = AVCaptureDevice.default(for: .video),
              let input = try? AVCaptureDeviceInput(device: device),
              session.canAddInput(input) else { return }
        session.addInput(input)

        let output = AVCaptureMetadataOutput()
        guard session.canAddOutput(output) else { return }
        session.addOutput(output)
        output.setMetadataObjectsDelegate(self, queue: .main)
        output.metadataObjectTypes = [.qr]

        let layer = AVCaptureVideoPreviewLayer(session: session)
        layer.videoGravity = .resizeAspectFill
        layer.frame = view.bounds
        view.layer.insertSublayer(layer, at: 0)
        preview = layer
    }

    private func startIfNeeded() {
        guard !session.isRunning, !session.inputs.isEmpty else { return }
        // startRunning blocks; keep it off the main thread.
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.session.startRunning()
        }
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        startIfNeeded()
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        if session.isRunning { session.stopRunning() }
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        preview?.frame = view.bounds
    }

    func metadataOutput(_ output: AVCaptureMetadataOutput,
                        didOutput metadataObjects: [AVMetadataObject],
                        from connection: AVCaptureConnection) {
        guard !didScan,
              let obj = metadataObjects.first as? AVMetadataMachineReadableCodeObject,
              obj.type == .qr,
              let value = obj.stringValue else { return }
        didScan = true
        session.stopRunning()
        UINotificationFeedbackGenerator().notificationOccurred(.success)
        onScan?(value)
    }
}
