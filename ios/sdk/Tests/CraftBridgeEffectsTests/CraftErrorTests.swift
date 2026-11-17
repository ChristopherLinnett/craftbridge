import CraftBridge
import Foundation
import Testing

@Suite struct CraftErrorTests {
    /// Each case reads in the words the core gives it, through `errorDescription` and through the
    /// `localizedDescription` an app shows for any error it caught.
    @Test func eachCaseReadsInTheCoresOwnWords() {
        let blur = #"no effect "Gausian Blur""#
        let words: [(CraftError, String)] = [
            (.InvalidParams(detail: "comp.new: unknown parameter(s) widht"), "comp.new: unknown parameter(s) widht"),
            (
                .CommandFailed(command: "effect.apply", detail: blur, step: 3),
                #"effect.apply: no effect "Gausian Blur" (step 3)"#
            ),
            (
                .CommandFailed(command: "effect.apply", detail: blur, step: nil),
                #"effect.apply: no effect "Gausian Blur""#
            ),
            (.NotFound(detail: ##"no layer "#9" in "Birthday Card""##), ##"no layer "#9" in "Birthday Card""##),
            (
                .Unsupported(detail: "external effect helpers are not available on iOS"),
                "external effect helpers are not available on iOS"
            ),
            (.Cancelled, "cancelled"),
            (
                .Io(detail: "cannot write /Card.ecproj: No space left on device"),
                "cannot write /Card.ecproj: No space left on device"
            ),
            (.Gpu(detail: "no Metal adapter"), "no Metal adapter"),
            (.Internal(detail: "engine is closed"), "engine is closed"),
        ]
        for (error, expected) in words {
            #expect(error.errorDescription == expected)
            #expect((error as any Error).localizedDescription == expected)
        }
    }
}
