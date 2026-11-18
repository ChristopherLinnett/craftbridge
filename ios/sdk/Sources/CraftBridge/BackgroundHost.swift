// What the controller needs of the running app, so a test can stand in for the system.

import Foundation
import UIKit

/// The app as the background controller sees it: the notifications that say it left the foreground, and
/// the system's grant of a little time to a task that must not be cut off.
package protocol BackgroundHost: Sendable {
    /// Where the app posts its lifecycle notifications.
    var notifications: NotificationCenter { get }
    /// Asks the system to keep the app running for a task named `name`, and answers the task's token, or
    /// nil when the system grants none (an app already past its time, or an extension). `onExpiry` is
    /// called when the system is about to stop the app, which is when the task must stop; the task still
    /// ends only through ``endTask(_:)``.
    @MainActor func beginTask(named name: String, onExpiry: @escaping @Sendable () -> Void) -> Int?
    /// Tells the system the task `token` names has ended.
    @MainActor func endTask(_ token: Int)
}

/// The host of a real app: `UIApplication`.
package struct UIKitHost: BackgroundHost {
    /// The Objective-C name of `UIApplication.shared`, written as text: naming the property, even in a key
    /// path, warns in an app extension's build, where it is unavailable. The tests hold it to the property.
    package static let sharedApplicationKey = "sharedApplication"

    /// The key the application is read by: ``sharedApplicationKey``, or another a test names to see a lookup fail.
    private let applicationKey: String

    package init(applicationKey: String = Self.sharedApplicationKey) {
        self.applicationKey = applicationKey
    }

    package var notifications: NotificationCenter { .default }

    /// The running app's `UIApplication`, read through Key-Value Coding, or nil where there is none: an
    /// extension, or a process that never started an app. `UIApplication.shared` itself does not compile in
    /// an app extension, which has none, and a package an extension links is built so. A key the class does
    /// not answer is nil too, since Key-Value Coding raises an Objective-C exception for an undefined key,
    /// which no Swift code can catch.
    @MainActor private var application: UIApplication? {
        guard UIApplication.responds(to: NSSelectorFromString(applicationKey)) else { return nil }
        return UIApplication.value(forKey: applicationKey) as? UIApplication
    }

    @MainActor package func beginTask(named name: String, onExpiry: @escaping @Sendable () -> Void) -> Int? {
        guard let task = application?.beginBackgroundTask(withName: name, expirationHandler: onExpiry),
            task != .invalid
        else { return nil }
        return task.rawValue
    }

    @MainActor package func endTask(_ token: Int) {
        application?.endBackgroundTask(UIBackgroundTaskIdentifier(rawValue: token))
    }
}
