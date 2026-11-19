// The screen: the preview, the two controls, the export and the share.

import PhotosUI
import SwiftUI
import UIKit

/// Postcard Studio's one screen.
struct StudioView: View {
    /// The studio the screen shows and drives.
    @Bindable var studio: Studio
    @State private var picked: PhotosPickerItem?

    var body: some View {
        VStack(spacing: 16) {
            PreviewView { studio.previewLayer = $0 }
                .aspectRatio(1, contentMode: .fit)
                .clipShape(RoundedRectangle(cornerRadius: 12))
                .accessibilityLabel("Preview of the card")
            HStack {
                Button(studio.playing ? "Pause" : "Play", systemImage: studio.playing ? "pause.fill" : "play.fill") {
                    studio.togglePlaying()
                }
                Slider(value: $studio.time, in: 0...Studio.cardSeconds)
            }
            HStack {
                PhotosPicker(selection: $picked, matching: .images) {
                    Label("Photo", systemImage: "photo")
                }
                TextField("Name", text: $studio.name)
                    .textFieldStyle(.roundedBorder)
            }
            if let note = studio.note {
                Text(note).font(.footnote).foregroundStyle(.secondary)
            }
            footer
            Spacer(minLength: 0)
        }
        .padding()
        .disabled(!studio.isUsable)
        .task { await studio.start() }
        .task(id: picked) {
            guard let picked else { return }
            await studio.choosePhoto { try await picked.loadTransferable(type: Data.self) }
        }
    }

    /// What sits under the controls: the export button, its progress, or the movie to share.
    @ViewBuilder private var footer: some View {
        switch studio.stage {
        case .starting:
            ProgressView("Starting the engine")
        case .unavailable(let reason):
            Text(reason).foregroundStyle(.red)
        case .ready:
            Button("Export the card", systemImage: "square.and.arrow.up") { Task { await studio.export() } }
                .buttonStyle(.borderedProminent)
        case .exporting(let fraction):
            ProgressView(value: fraction) { Text("Exporting") }
            Button("Cancel", role: .cancel) { studio.cancelExport() }
        case .exported(let movie):
            Button("Share the card", systemImage: "square.and.arrow.up") { studio.share() }
                .buttonStyle(.borderedProminent)
                .sheet(isPresented: $studio.sharing) {
                    ShareSheet(items: [movie]) { studio.sharing = false }
                        .presentationDetents([.medium, .large])
                }
            Button("Export again") { Task { await studio.export() } }
        case .failed(let reason):
            Text(reason).foregroundStyle(.red)
            Button("Export again") { Task { await studio.export() } }
        }
    }
}

/// The system's share sheet, offering `items`. It is shown from the studio's own state, `Studio.sharing`, so
/// whatever opens the sheet opens it the one way the Share button does.
struct ShareSheet: UIViewControllerRepresentable {
    /// What the sheet offers: the exported movie.
    let items: [URL]
    /// Called once the person has shared or closed the sheet, which then has nothing left to show.
    let onEnd: @MainActor () -> Void

    func makeUIViewController(context: Context) -> UIActivityViewController {
        let sheet = UIActivityViewController(activityItems: items, applicationActivities: nil)
        sheet.completionWithItemsHandler = { _, _, _, _ in
            Task { @MainActor in onEnd() }
        }
        return sheet
    }

    func updateUIViewController(_ sheet: UIActivityViewController, context: Context) {}
}
