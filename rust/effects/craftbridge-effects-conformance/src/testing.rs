//! What the tests of this crate share: images to compare.

/// A PNG of `width` x `height`, each pixel's colour given by `shade`, fully opaque.
pub(crate) fn png_bytes(width: u32, height: u32, shade: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let [r, g, b] = shade(x, y);
            pixels.extend([r, g, b, 255]);
        }
    }
    let mut bytes = Vec::new();
    image::write_buffer_with_format(&mut std::io::Cursor::new(&mut bytes), &pixels, width, height, image::ExtendedColorType::Rgba8, image::ImageFormat::Png)
        .expect("encode");
    bytes
}
