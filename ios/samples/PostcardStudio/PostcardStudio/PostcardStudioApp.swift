// Postcard Studio, the sample app of plan §5: a birthday card for Sam, made on the phone with CraftBridge.

import SwiftUI

/// The app: one window holding the studio.
@main
struct PostcardStudioApp: App {
    /// The app's studio, made once so every scene shares one engine.
    @State private var studio = Studio()

    /// Registers the export's background task before the app finishes launching, which the system requires of
    /// every task it may start later.
    init() {
        ExportRecipe.registerBackgroundTask()
    }

    var body: some Scene {
        WindowGroup {
            StudioView(studio: studio)
                #if DEBUG
                    .task { await Spine.runIfAsked(on: studio) }
                #endif
        }
    }
}
