import SwiftUI

@main
struct FontProbeApp: App {
    init() {
        // Run the probe synchronously at launch so the report is on stdout/os_log
        // and in Documents before the first frame is on screen.
        FontProbe.runAndReport()
    }

    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}
