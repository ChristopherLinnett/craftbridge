import SwiftUI

@main
struct FontProbeApp: App {
    init() {
        // Run the probe synchronously at launch so the report is on stdout and in Documents
        // (os_log gets a one-line pointer) before the first frame is on screen.
        FontProbe.runAndReport()
    }

    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}
