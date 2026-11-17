//! What the tests of this crate share: images to compare and programs to run.

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

/// An executable script that runs `body`, written in `folder`: a stand-in for a side, started the way a
/// side is.
#[cfg(unix)]
pub(crate) fn executable(folder: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let binary = folder.join(name);
    std::fs::write(&binary, format!("#!/bin/sh\n{body}\n")).expect("write the script");
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).expect("make it executable");
    binary
}

/// Holds that the environment a side printed (`env`'s output) is its configuration folder, `dir`'s own, and
/// nothing of the runner's: whatever this process holds (HOME, and here cargo's own variables) stays
/// behind. Only the names the shell running the script exports for itself are left out of the comparison.
#[cfg(unix)]
pub(crate) fn assert_only_the_configuration_folder(printed: &str, dir: &std::path::Path) {
    let shells_own = ["PWD", "OLDPWD", "SHLVL", "_"];
    let given: Vec<&str> = printed.lines().filter(|line| !shells_own.iter().any(|name| line.starts_with(&format!("{name}=")))).collect();
    let wanted = format!("{}={}", crate::oracle::CONFIG_VARIABLE, crate::oracle::config_folder(dir).display());
    assert_eq!(given, [wanted], "{printed}");
    assert!(std::env::var_os("HOME").is_some(), "the runner's own environment holds what must not reach a side");
}
