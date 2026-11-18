// What an app's engine does with the GPU, and with an export under way, when the app leaves the foreground.

/// What an engine does with its GPU when the app enters the background.
///
/// Metal refuses the command buffers of an app that is not in the foreground, unless it runs a
/// `BGContinuedProcessingTask` with the GPU entitlement (`evidence/05-mobile-platforms.md` Finding 19), so
/// by default the engine gives its device up and renders on the CPU until the app returns.
public enum BackgroundPolicy: Sendable, Equatable {
    /// Gives the GPU up on entering the background (``Engine/suspendGpu()``) and asks for it again on
    /// return to the foreground (``Engine/resumeGpu()``). Every render started in between answers on the
    /// CPU. This is the default.
    case cpuWhenBackgrounded
    /// Keeps the GPU through the background, for an app running a `BGContinuedProcessingTask` with the
    /// `com.apple.developer.background-tasks.continued-processing.gpu` entitlement on a device that
    /// supports it. An app that sets it without the entitlement gets Metal's refusals.
    case keepGpu

    /// What an engine does when the app says nothing: ``cpuWhenBackgrounded``.
    public static let `default` = BackgroundPolicy.cpuWhenBackgrounded
}
