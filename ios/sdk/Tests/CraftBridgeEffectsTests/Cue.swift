// The movie a person watches to judge the running example's sound against its pictures, made once here: the
// Photos test saves it to the phone for the owner, and a simulator test reads the same movie back, so what the
// owner judges is pinned off the phone too. Its pictures flash white from the frame where the sound's tone
// begins, since a card that shows nothing then gives the eye nothing to match the tone to.

import CraftBridge
import CraftBridgeEffects
import Foundation

/// The first frame of the flash: the one the tone begins in, after the sound's ``silentSeconds`` of silence.
let cueFrame = silentSeconds * cardFrameRate

/// How many frames the flash lasts: long enough to see, short enough to read as the tone's start.
let cueFrames = 6

/// Adds the flash to `comp`: a full-frame white layer from frame ``cueFrame`` for ``cueFrames`` frames.
func addCue(to comp: ItemReference, in engine: EffectsEngine) async throws {
    let perSecond = Double(cardFrameRate)
    _ = try await engine.run([
        (command: "layer.newSolid", params: ["comp": comp.jsonValue, "name": "Cue", "color": "#ffffff"]),
        (
            command: "layer.timing",
            params: [
                "layers": ["Cue"], "in": .number(Double(cueFrame) / perSecond),
                "out": .number(Double(cueFrame + cueFrames) / perSecond),
            ]
        ),
    ])
}

/// The movie to watch, exported into `folder` through the hardware path: the running example's card at its own
/// size, 1080 by 1080, filled in as Sam's, with a sound as long as the card, ``silentSeconds`` of silence and then
/// the tone, and the flash. Answers the movie's file.
func exportTheMovieToWatch(_ engine: EffectsEngine, in folder: URL) async throws -> URL {
    let template = try await makeCardTemplate(engine, in: folder, width: 1080, height: 1080)
    let sam = folder.appending(path: "sam.png")
    let sound = folder.appending(path: "tone.wav")
    try picture(.green, to: sam)
    try tone(seconds: cardSeconds).write(to: sound)

    let card = try await engine.templates.open(template)
    try await card.set("Name", "Sam")
    try await card.set("Photo", media: sam)
    _ = try await engine.run([
        (command: "file.import", params: ["paths": [.string(sound.path(percentEncoded: false))]]),
        (command: "layer.addItem", params: ["comp": card.comp.jsonValue, "item": "tone.wav", "time": 0]),
    ])
    try await addCue(to: card.comp, in: engine)
    let movie = folder.appending(path: "card.mp4")
    _ = try await engine.export(card.comp, to: movie, format: .h264, extra: ["hardware": true]).result
    return movie
}
