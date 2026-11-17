//! What the crate's tests share: how long a test waits for what the engine must do in the end,
//! the small handshakes every test that races the engine needs, and the host GPU and textures the
//! tests that draw use.

use crate::Result;
use crate::engine::{Engine, Message};
use crate::folders::Folders;
use crate::gpu::{GpuChoice, GpuDevice};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How long a test waits for anything the engine must eventually deliver, so a broken engine fails
/// with a message instead of hanging the suite. No behaviour of the engine is measured against it:
/// every promise is proven by what arrives, and by what does not arrive until the test lets it,
/// never by how long something took, so a loaded machine slows the tests without failing them.
pub(crate) const DEADLINE: Duration = Duration::from_secs(10);

/// Whether `done` came true within [`DEADLINE`], asked again every millisecond until it does.
pub(crate) fn eventually(mut done: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + DEADLINE;
    while !done() {
        if Instant::now() > until {
            return false;
        }
        thread::sleep(Duration::from_millis(1));
    }
    true
}

/// A gate one side of a test opens and the other waits at: a fake app's call or drop held until
/// the test lets it go, or the test held until the app has got somewhere. Nothing opens on a timer,
/// so what a test checks while a gate is shut holds however slowly the machine runs.
#[derive(Default)]
pub(crate) struct Gate {
    open: AtomicBool,
}

impl Gate {
    /// Let whoever waits at the gate go on.
    pub(crate) fn open(&self) {
        self.open.store(true, Ordering::SeqCst);
    }

    /// Whether the gate opened within [`DEADLINE`]: false means it never did, which the waiter
    /// answers as a failure rather than waiting for ever.
    pub(crate) fn wait(&self) -> bool {
        eventually(|| self.open.load(Ordering::SeqCst))
    }
}

/// Hand one plain call to `engine` from the test's own thread and take the channel its answer
/// comes on: the inbox is first in, first out, so calls handed over one after another from one
/// thread reach the engine in that order, with no thread to schedule.
pub(crate) fn queue_call(engine: &Engine, method: &str, params: Value) -> oneshot::Receiver<Result<Value>> {
    engine.submit(Message::request(method, params)).unwrap()
}

/// What a test waits on for an answer: the oneshot channel the engine answers a call on, or a channel
/// of the test's own that a thread making the call answers on.
pub(crate) trait Arrives<T> {
    /// The answer, or none if it did not arrive within `timeout`.
    fn within(&self, timeout: Duration) -> Option<T>;
}

impl<T> Arrives<T> for oneshot::Receiver<T> {
    fn within(&self, timeout: Duration) -> Option<T> {
        self.recv_timeout(timeout).ok()
    }
}

impl<T> Arrives<T> for mpsc::Receiver<T> {
    fn within(&self, timeout: Duration) -> Option<T> {
        self.recv_timeout(timeout).ok()
    }
}

/// Take the answer that arrives on `rx`, failing the test at the deadline if none ever does: an
/// engine that stops answering is a test failure with a message, never a hung suite.
pub(crate) fn recv<T>(rx: &impl Arrives<T>) -> T {
    rx.within(DEADLINE).unwrap_or_else(|| panic!("the call was never answered"))
}

/// Close `engine` from a thread of its own and return once the close has begun, so `Close` is
/// queued and every export has been asked to stop. The close runs apart from the test because it
/// joins the engine thread, which may still be inside a call only the test can let go.
pub(crate) fn close_begun(engine: &Arc<Engine>) -> JoinHandle<Result<()>> {
    let closing = engine.clone();
    let closer = thread::spawn(move || closing.close());
    assert!(eventually(|| engine.is_closing()), "the close never began");
    closer
}

/// The folders of an app's first launch with its sandbox at `container`: the working folders inside it,
/// as an app names them, none created yet.
pub(crate) fn scratch_folders(container: PathBuf) -> Folders {
    Folders {
        config: container.join("config"),
        cache: container.join("cache"),
        models: container.join("models"),
        container,
        bundle: None,
        plugins: None,
        fonts: Vec::new(),
    }
}

/// The host's GPU, which the tests that draw run on.
pub(crate) struct Rig {
    pub(crate) adapter: wgpu::Adapter,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
}

/// The host's GPU, asked as the engine asks for it. The machines the gate runs on have one (Metal),
/// so a host without one fails the tests that draw instead of passing what they never proved.
pub(crate) fn rig() -> Rig {
    match GpuDevice::request(GpuChoice::Preferred) {
        GpuDevice::Device { adapter, device, queue } => Rig { adapter, device, queue },
        GpuDevice::CpuOnly { reason } => panic!("the host must answer a GPU for this test to mean anything: {reason}"),
    }
}

/// A 2D texture of `levels` mip levels, its first `size` pixels across.
pub(crate) fn texture(
    device: &wgpu::Device,
    (width, height): (u32, u32),
    levels: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test texture"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// Read the first level of `texture`, made with `COPY_SRC` in an 8-bit four-channel format, back as
/// straight RGBA bytes, row by row, whatever channel order the format stores.
pub(crate) fn read_back(rig: &Rig, texture: &wgpu::Texture) -> Vec<[u8; 4]> {
    let (width, height) = (texture.width(), texture.height());
    let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let readback = rig.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test readback"),
        size: u64::from(row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = rig.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(height) },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    rig.queue.submit([encoder.finish()]);
    let (sender, receiver) = mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    rig.device.poll(wgpu::PollType::wait_indefinitely()).expect("the device finishes the readback");
    receiver.recv().expect("the map answers").expect("the readback maps");
    let bytes = readback.slice(..).get_mapped_range().expect("the mapped range");
    let bgra = texture.format() == wgpu::TextureFormat::Bgra8Unorm;
    let mut pixels = Vec::new();
    for y in 0..height as usize {
        for x in 0..width as usize {
            let at = y * row as usize + x * 4;
            let px = &bytes[at..at + 4];
            pixels.push(if bgra { [px[2], px[1], px[0], px[3]] } else { [px[0], px[1], px[2], px[3]] });
        }
    }
    pixels
}
