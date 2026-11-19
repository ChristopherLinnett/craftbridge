// The photo a person picks, made into the file the card's Photo control takes.

import Foundation
import UIKit

/// The side, in pixels, of the square a picked photo is cut to, which is how large it shows on the 1080-pixel
/// card: the Photo control swaps the picture and keeps the layer's transform, which holds a replacement's
/// top-left corner where the template's 32-pixel placeholder has its own and shows it at its own size.
let postcardPhotoSide = 400

/// A reason a picked photo cannot be used.
enum PostcardPhotoError: Error, LocalizedError {
    /// The bytes are no picture the system can read.
    case unreadable
    /// The picture could not be written for the engine to read.
    case unwritable

    var errorDescription: String? {
        switch self {
        case .unreadable: "That photo could not be read."
        case .unwritable: "That photo could not be prepared for the card."
        }
    }
}

/// Cuts the picture in `data` to a `postcardPhotoSide` square around its centre and writes it as a JPEG in the
/// app's temporary folder, which is inside the container the engine reads from. The engine reads files, not
/// the picker's data, so the card is given the path of this one.
func postcardPhoto(from data: Data) throws -> URL {
    guard let picture = UIImage(data: data) else { throw PostcardPhotoError.unreadable }
    let side = CGFloat(postcardPhotoSide)
    let format = UIGraphicsImageRendererFormat()
    format.scale = 1
    let shortest = min(picture.size.width, picture.size.height)
    guard shortest > 0 else { throw PostcardPhotoError.unreadable }
    let scale = side / shortest
    let drawn = CGSize(width: picture.size.width * scale, height: picture.size.height * scale)
    let square = UIGraphicsImageRenderer(size: CGSize(width: side, height: side), format: format).image { _ in
        picture.draw(
            in: CGRect(
                x: (side - drawn.width) / 2, y: (side - drawn.height) / 2, width: drawn.width, height: drawn.height))
    }
    guard let jpeg = square.jpegData(compressionQuality: 0.9) else { throw PostcardPhotoError.unwritable }
    let file = URL.temporaryDirectory.appending(path: "photo-\(UUID().uuidString).jpg")
    do {
        try jpeg.write(to: file)
    } catch {
        throw PostcardPhotoError.unwritable
    }
    return file
}
