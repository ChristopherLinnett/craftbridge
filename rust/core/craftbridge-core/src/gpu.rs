//! The engine's GPU device (plan §17, decision D-7).
//!
//! The core creates the wgpu device itself rather than letting the compositor pick one, because
//! the compositor's own request failed where the plugin runs: the iOS Simulator's Metal device
//! offers fewer limits than `wgpu::Limits::default()` asks for, and the Android emulator's best
//! adapter was its OpenGL ES translator, which has no compute shaders
//! (`plan/evidence/03-device-feasibility.md` Findings 6–8). So the backend is chosen per platform
//! — Metal on iOS, Vulkan on Android, the host's on the desktop — GL is never requested, and the
//! requested limits are the defaults clamped to what the adapter offers.
//!
//! A device answers `(adapter, device, queue)` for the app crate to hand to
//! `effectcraft_gpu::Gpu::new`; no device answers why, in words `info.gpu` can report, and every
//! render then runs on the CPU (L7). A failed adapter or device request is that answer, never a
//! panic. A device also carries its failure notice ([`GpuFailure`], §17): the app crate's handlers
//! for wgpu's device-lost and uncaptured-error callbacks report into it, and the GPU is dropped on
//! its first report.

use std::sync::{Arc, OnceLock};

use crate::Result;
use crate::error::CraftError;

/// What the app asked for when it started the engine (plan §14). `Preferred` takes a GPU when
/// there is one; `CpuOnly` never asks, and everything renders on the CPU (L7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuChoice {
    /// Ask the platform's backend for a device; fall back to the CPU only when there is none.
    Preferred,
    /// Render on the CPU without asking.
    CpuOnly,
}

/// The platform the core picks GPU backends for, and the platform an app plugin picks its
/// per-platform rules by (§19): the same three names answer both. A build takes the one its target
/// names, and the tests name each target, so every platform's pick is proven on the machine the
/// tests run on while the gate's phone builds prove the code compiles for the phones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Ios,
    Android,
    /// Any other target, a desktop: its own backends, Metal on macOS.
    Host,
}

impl Platform {
    /// The platform this build was compiled for.
    #[must_use]
    pub fn current() -> Platform {
        Platform::of(std::env::consts::OS)
    }

    /// The platform of a target operating system, named as `std::env::consts::OS` names it (the
    /// target's `target_os`; the iOS Simulator's is `ios`). A plain function of the name, rather than
    /// a `cfg!` a host build always folds to `Host`, so the tests pin each phone's pick on the host.
    #[must_use]
    pub fn of(os: &str) -> Platform {
        match os {
            "ios" => Platform::Ios,
            "android" => Platform::Android,
            _ => Platform::Host,
        }
    }

    /// The wgpu backends to ask, never GL: the compositor needs compute shaders, which the
    /// emulator's GL lacked (D-7, `plan/evidence/03-device-feasibility.md` Finding 7). On the
    /// host the machine's own backends answer, minus GL, so a desktop test drives the same rule.
    fn backends(self) -> wgpu::Backends {
        match self {
            Platform::Ios => wgpu::Backends::METAL,
            Platform::Android => wgpu::Backends::VULKAN,
            Platform::Host => wgpu::Backends::all().difference(wgpu::Backends::GL),
        }
    }
}

/// A device's failure notice (plan §17): where wgpu's device-lost and uncaptured-error callbacks
/// carry their reason, and what is read to drop the GPU on the first report. The first report's
/// words are the ones kept — a failing device can report again, with every later uncaptured error
/// and with the loss that follows one, and only the first failure names why the render went back
/// to the CPU. Calling it is safe from any thread, since the callbacks that call it are wgpu's own,
/// and harmless once the device is gone.
#[derive(Clone, Debug, Default)]
pub struct GpuFailure(Arc<OnceLock<String>>);

impl GpuFailure {
    /// Report a device failure in words, as `info.gpu` will report it. Whether this report was the
    /// first: once a failure is in, the GPU is dropped and a later report changes nothing.
    pub fn report(&self, reason: String) -> bool {
        self.0.set(reason).is_ok()
    }

    /// Whether a failure was reported.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.0.get().is_some()
    }

    /// The first failure's words, if one was reported.
    #[must_use]
    pub fn reason(&self) -> Option<String> {
        self.0.get().cloned()
    }
}

/// The engine's GPU: the device to render on, or the reason the render runs on the CPU instead
/// (§17's *GPU attached* and *CPU only* states).
#[derive(Debug)]
pub enum GpuDevice {
    /// A device on the platform's backend. `adapter`, `device` and `queue` go together to
    /// `effectcraft_gpu::Gpu::new` when the engine attaches the GPU (§17).
    Device {
        /// The instance the adapter and device were made on. A surface for a preview is made on
        /// this instance and no other (§20): one made on a second instance would name adapters
        /// this device does not know, and a wgpu surface and a wgpu device must come from one.
        instance: wgpu::Instance,
        /// The adapter the device was requested from; `info.gpu` names it.
        adapter: wgpu::Adapter,
        /// The logical device the compositor renders on.
        device: wgpu::Device,
        /// The queue that carries the device's work.
        queue: wgpu::Queue,
        /// The device's failure notice: the one handle every holder of this device shares (§17).
        failure: GpuFailure,
    },
    /// No device — `CpuOnly` was chosen, no adapter answered, or the request failed. `reason` is
    /// in words `info.gpu` can report, and every render runs on the CPU (L7).
    CpuOnly {
        /// Why there is no device, as `info.gpu` reports it.
        reason: String,
    },
}

impl GpuDevice {
    /// Ask for the device the app's [`GpuChoice`] allows. This blocks until the adapter and
    /// device futures answer, on the executor EffectCraft itself blocks on; call it on the
    /// engine's own thread, not the app's UI thread.
    #[must_use]
    pub fn request(choice: GpuChoice) -> GpuDevice {
        match choice {
            GpuChoice::Preferred => on_backends(Platform::current().backends()),
            GpuChoice::CpuOnly => GpuDevice::CpuOnly { reason: "the app asked for CPU-only rendering".to_string() },
        }
    }

    /// What `info.gpu` reports (§14's `info`): `"<adapter> (<backend>)"` with a device, or the
    /// reason the render is on the CPU. The device is named as the compositor names the device it
    /// renders on, `Apple M5 Max (Metal)` (`effectcraft: crates/gpu/src/context.rs:866`), so
    /// `info.gpu` and the compositor's own report spell one device alike.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            GpuDevice::Device { adapter, .. } => {
                let info = adapter.get_info();
                format!("{} ({:?})", info.name, info.backend)
            }
            GpuDevice::CpuOnly { reason } => reason.clone(),
        }
    }

    /// The trio for `effectcraft_gpu::Gpu::new`, or nothing while the render is on the CPU (§17).
    /// `Gpu::new` takes the adapter by reference and the device and queue by value; wgpu's handles
    /// are reference-counted, so the caller hands it clones and this device keeps its own.
    #[must_use]
    pub fn parts(&self) -> Option<(&wgpu::Adapter, &wgpu::Device, &wgpu::Queue)> {
        match self {
            GpuDevice::Device { adapter, device, queue, .. } => Some((adapter, device, queue)),
            GpuDevice::CpuOnly { .. } => None,
        }
    }

    /// The instance the device was made on, or nothing while the render is on the CPU: the one a
    /// platform surface must be made on, as [`Self::parts`] names the device that surface will
    /// show its frames with (§20).
    #[must_use]
    pub fn instance(&self) -> Option<&wgpu::Instance> {
        match self {
            GpuDevice::Device { instance, .. } => Some(instance),
            GpuDevice::CpuOnly { .. } => None,
        }
    }

    /// The device's failure notice (§17): the one handle every holder of this device shares, so
    /// the app crate's handlers for wgpu's device-lost and uncaptured-error callbacks report into it
    /// and every holder of the device sees the same first report. `CpuOnly` has no device to fail:
    /// the notice it hands out never fails, and there is no GPU to drop.
    #[must_use]
    pub fn device_failure_notifier(&self) -> GpuFailure {
        match self {
            GpuDevice::Device { failure, .. } => failure.clone(),
            GpuDevice::CpuOnly { .. } => GpuFailure::default(),
        }
    }
}

/// Every kind of error a wgpu device reports. wgpu hands an error no open scope captures to its
/// default handler, which panics, so [`validated`] opens a scope for each kind.
const CAPTURED: [wgpu::ErrorFilter; 3] = [wgpu::ErrorFilter::Validation, wgpu::ErrorFilter::OutOfMemory, wgpu::ErrorFilter::Internal];

/// Run `make`, which works on `device` (creating objects, recording, submitting, acquiring or
/// presenting a surface's texture), and answer [`CraftError::Gpu`] when the device reports any
/// error while it ran. Without an open scope wgpu's answer to such an error is a panic, which
/// AGENTS.md §3 forbids, so all the core's work on a device goes through this. Scopes are
/// per thread, which is the engine's one thread, and a scope pops only from the top of the stack,
/// so they are popped in the reverse of the order they were pushed.
pub(crate) fn validated<T>(device: &wgpu::Device, what: &str, make: impl FnOnce() -> T) -> Result<T> {
    let scopes = CAPTURED.map(|filter| device.push_error_scope(filter));
    let made = make();
    let mut reported = None;
    for scope in scopes.into_iter().rev() {
        reported = reported.or(pollster::block_on(scope.pop()));
    }
    match reported {
        Some(error) => Err(CraftError::gpu(format!("{what}: {error}"))),
        None => Ok(made),
    }
}

/// `max_texture_dimension_2d` is never asked above this, as the compositor caps it
/// (`effectcraft: crates/gpu/src/context.rs:1430-1435`). A comp larger than the device's texture
/// limit renders on the CPU for that comp — the compositor's own decline path (§17).
const MAX_TEXTURE_DIMENSION_2D: u32 = 16_384;

/// The limits the compositor's own device request asks of an adapter, unclamped: the two it takes
/// from the adapter itself, `max_texture_dimension_2d` capped at [`MAX_TEXTURE_DIMENSION_2D`] and
/// `max_buffer_size` as the adapter gives it, and `wgpu::Limits::default()` for every other limit,
/// whatever the adapter offers (`effectcraft: crates/gpu/src/context.rs:1430-1435`). An adapter that
/// offers less than the default refuses this request, which is why the core never makes it.
fn compositor_limits(adapter_limits: &wgpu::Limits) -> wgpu::Limits {
    wgpu::Limits {
        max_texture_dimension_2d: adapter_limits.max_texture_dimension_2d.min(MAX_TEXTURE_DIMENSION_2D),
        max_buffer_size: adapter_limits.max_buffer_size,
        ..wgpu::Limits::default()
    }
}

/// The limits to request from an adapter: the compositor's own, clamped to the adapter's, so a
/// device that offers less than the default still answers (the iOS Simulator's Metal device offers
/// 15 inter-stage shader variables against the default's 16) (D-7, §17).
fn required_limits(adapter_limits: &wgpu::Limits) -> wgpu::Limits {
    compositor_limits(adapter_limits).or_worse_values_from(adapter_limits)
}

/// The device descriptor: the clamped limits and the memory-lean buffers, as the compositor's own
/// request keeps them (`effectcraft: crates/gpu/src/context.rs:816-818`).
fn device_descriptor(required_limits: wgpu::Limits) -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor { label: Some("craftbridge core"), required_limits, memory_hints: wgpu::MemoryHints::MemoryUsage, ..Default::default() }
}

/// The instance asking exactly these backends. Deliberately not built `with_env()`: the backend is
/// the choice this module makes per platform, and the environment must not switch it to GL.
fn instance(backends: wgpu::Backends) -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor { backends, ..wgpu::InstanceDescriptor::new_without_display_handle() })
}

/// The device on the best adapter of `backends`, or the reason there is none: no adapter
/// answered, or the device request failed. Both answers say why in words `info.gpu` can report,
/// and the render runs on the CPU (L7).
fn on_backends(backends: wgpu::Backends) -> GpuDevice {
    let instance = instance(backends);
    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        // The compositor's own pick when `WGPU_POWER_PREF` is unset: a discrete GPU, then an
        // integrated one, above a virtual or a software one (`effectcraft: crates/gpu/src/context.rs:340-347`).
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    })) {
        Ok(adapter) => adapter,
        // wgpu's own error names each backend asked and why it gave none, as the compositor reports it.
        Err(error) => {
            return GpuDevice::CpuOnly { reason: format!("no adapter: {error}") };
        }
    };
    let limits = required_limits(&adapter.limits());
    open(&instance, adapter, limits)
}

/// The device on this adapter with these limits, asked of the adapter. A refused request — the
/// limits more than it gives, the features more than it has — is the reason `info.gpu` reports
/// and the CPU renders, never a panic (§17's *CPU only* state).
fn open(instance: &wgpu::Instance, adapter: wgpu::Adapter, required: wgpu::Limits) -> GpuDevice {
    match pollster::block_on(adapter.request_device(&device_descriptor(required))) {
        Ok((device, queue)) => GpuDevice::Device { instance: instance.clone(), adapter, device, queue, failure: GpuFailure::default() },
        Err(error) => GpuDevice::CpuOnly { reason: format!("no device on {}: {error}", adapter.get_info().name) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The adapter a device test ran on, as one line on stderr, which a run with `--nocapture`
    /// shows: the adapter, its backend and type and its driver, so a run on a phone or an emulator
    /// records what it ran on.
    fn say_adapter(what: &str, adapter: &wgpu::Adapter) {
        let info = adapter.get_info();
        // A driver's own description can run over several lines, and Metal gives none; the record is one.
        let driver =
            std::iter::once(info.driver.as_str()).chain(info.driver_info.lines()).map(str::trim).filter(|part| !part.is_empty()).collect::<Vec<_>>().join("; ");
        let driver = if driver.is_empty() { "none given" } else { driver.as_str() };
        eprintln!("{what}: {} ({:?}), {:?}, driver: {driver}", info.name, info.backend, info.device_type);
    }

    /// Each limit `asked` asks more of than `offered` gives, in words, read by wgpu's own comparison
    /// of every limit: what a device request for `asked` on an adapter offering `offered` is refused for.
    fn limits_above(asked: &wgpu::Limits, offered: &wgpu::Limits) -> Vec<String> {
        let mut above = Vec::new();
        asked.check_limits_with_fail_fn(offered, false, |name, asked, offered| above.push(format!("{name}: asked {asked}, offered {offered}")));
        above
    }

    /// The production request, which is what the acceptance runs, on whatever machine runs it (the
    /// desktop, the iPhone, a phone or an emulator): it must answer a device, on a backend the
    /// platform asks for. It drives the whole production path — instance, adapter pick, clamped
    /// limits, device request — so the clamping and the never-GL rule are proven on a real device
    /// (V4), and the phone code paths around them are the same code.
    #[test]
    fn this_machines_request_answers_a_device_with_clamped_limits() {
        let rig = crate::testing::rig();
        let adapter_limits = rig.adapter.limits();
        let device_limits = rig.device.limits();
        say_adapter("V4, the clamped request was given", &rig.adapter);
        eprintln!("V4, the device's limits: {device_limits:?}");
        // Every limit is exactly what `required_limits` computes from the adapter — the clamp
        // itself, end to end.
        assert_eq!(device_limits, required_limits(&adapter_limits));
        // No limit asks more than the adapter gives.
        let beyond = limits_above(&device_limits, &adapter_limits);
        assert!(beyond.is_empty(), "limits above the adapter's: {beyond:?}");
        // The two limits the adapter supplies on its own (§17): `max_buffer_size` as given, and
        // the texture dimension capped at 16384 (an adapter above the cap is clamped to it).
        assert_eq!(device_limits.max_buffer_size, adapter_limits.max_buffer_size);
        assert_eq!(device_limits.max_texture_dimension_2d, adapter_limits.max_texture_dimension_2d.min(16_384));
        // The backend is one the platform asks for, never GL.
        let backend = rig.adapter.get_info().backend;
        assert!(Platform::current().backends().contains(wgpu::Backends::from(backend)), "{backend:?} is not a backend {:?} asks for", Platform::current());
        assert_ne!(backend, wgpu::Backend::Gl);
    }

    /// Whether the adapter this machine's request picked also takes the compositor's own limits,
    /// unclamped (V4): it takes them exactly when it offers every one, and its refusal is words
    /// otherwise, never a panic. The outcome is recorded either way, with the limits the adapter
    /// offers less of than the default asks, which is why the clamp exists.
    #[test]
    fn the_adapter_takes_the_compositors_unclamped_limits_exactly_when_it_offers_them() {
        let rig = crate::testing::rig();
        let adapter_limits = rig.adapter.limits();
        let asked = compositor_limits(&adapter_limits);
        let short = limits_above(&asked, &adapter_limits);
        let given = pollster::block_on(rig.adapter.request_device(&device_descriptor(asked)));
        say_adapter("V4, the compositor's default limits, unclamped", &rig.adapter);
        match &given {
            Ok(_) => eprintln!("V4, the compositor's default limits: given"),
            Err(refusal) => eprintln!("V4, the compositor's default limits: refused ({refusal}); the adapter offers less than they ask: {short:?}"),
        }
        assert_eq!(given.is_ok(), short.is_empty(), "the request's outcome must follow the limits the adapter offers: {given:?}, short of {short:?}");
    }

    /// The clamp itself on numbers a real host may not offer: a simulator-like adapter with fewer
    /// inter-stage variables than the default and a coarser storage-buffer alignment, more bind
    /// groups and storage buffers than the default asks, a texture dimension above the cap, and a
    /// buffer size the default would not ask for. A mutation of the cap, of the adapter buffer
    /// size, of the clamp, or of the defaults it starts from fails this.
    #[test]
    fn limits_clamp_every_value_to_the_adapter_and_cap_the_two() {
        let default = wgpu::Limits::default();
        let adapter = wgpu::Limits {
            max_inter_stage_shader_variables: 15,     // the iOS Simulator's shortfall (Finding 6)
            min_storage_buffer_offset_alignment: 512, // an alignment worse than the default's 256
            max_bind_groups: 8,                       // better than the default: not asked for
            max_storage_buffers_per_shader_stage: 29, // the same
            max_texture_dimension_2d: 163_840,        // above the cap
            max_buffer_size: 64 << 30,                // 64 GiB, more than the default asks
            ..default
        };
        let requested = required_limits(&adapter);
        // Every limit at once: the default where the adapter offers more, the adapter's where it
        // offers less, and the two the compositor takes from the adapter — the texture dimension
        // capped at 16384, the number §17 fixes, written as a literal here so a mutation of the
        // constant cannot agree with itself and hide, and the buffer size as the adapter gives it.
        let expected = wgpu::Limits {
            max_inter_stage_shader_variables: 15,
            min_storage_buffer_offset_alignment: 512,
            max_texture_dimension_2d: 16_384,
            max_buffer_size: 64 << 30,
            ..default
        };
        assert_eq!(requested, expected);
        // An adapter under the cap keeps its own texture dimension.
        let small = required_limits(&wgpu::Limits { max_texture_dimension_2d: 8192, ..default });
        assert_eq!(small.max_texture_dimension_2d, 8192);
        // Nothing requested exceeds its adapter.
        assert!(requested.max_buffer_size <= adapter.max_buffer_size);
        assert!(requested.max_texture_dimension_2d <= adapter.max_texture_dimension_2d);
    }

    /// The compositor's own request is wgpu's defaults but for the two limits it takes from the
    /// adapter, whatever the adapter offers of the rest: an adapter with less than the default keeps
    /// the default here, which is what makes the request one it can refuse. The cap is written as a
    /// literal so a mutation of the constant cannot agree with itself.
    #[test]
    fn the_compositors_limits_are_the_defaults_but_for_the_two_the_adapter_supplies() {
        let default = wgpu::Limits::default();
        let adapter = wgpu::Limits { max_inter_stage_shader_variables: 15, max_texture_dimension_2d: 163_840, max_buffer_size: 64 << 30, ..default };
        let asked = compositor_limits(&adapter);
        assert_eq!(asked, wgpu::Limits { max_texture_dimension_2d: 16_384, max_buffer_size: 64 << 30, ..default });
        assert_eq!(asked.max_inter_stage_shader_variables, 16, "the default, though the adapter offers 15");
        assert_eq!(
            limits_above(&asked, &adapter),
            ["max_inter_stage_shader_variables: asked 16, offered 15"],
            "an adapter with fewer inter-stage variables than the default refuses it for that limit alone"
        );
        assert_eq!(limits_above(&required_limits(&adapter), &adapter), Vec::<String>::new(), "the clamped request is one the adapter takes");
    }

    /// `CpuOnly` never asks and answers the reason `info.gpu` can show.
    #[test]
    fn cpu_only_choice_answers_a_reason() {
        let gpu = GpuDevice::request(GpuChoice::CpuOnly);
        let GpuDevice::CpuOnly { reason } = &gpu else {
            panic!("CpuOnly must not take a device");
        };
        assert!(reason.contains("CPU-only"), "reason reads: {reason}");
        assert!(gpu.parts().is_none());
        // The reason is exactly what `info.gpu` would report (§17: the CPU-only state shows it).
        assert_eq!(gpu.summary(), reason.clone());
    }

    /// A request that finds no adapter answers the reason, not a panic — proven by asking a
    /// backend set that can hold none, which runs the production no-adapter branch. The reason
    /// carries wgpu's own error, which says why each backend gave none.
    #[test]
    fn no_adapter_answers_a_reason() {
        let gpu = on_backends(wgpu::Backends::empty());
        let GpuDevice::CpuOnly { reason } = &gpu else {
            panic!("an empty backend set cannot answer a device: {}", gpu.summary());
        };
        let Err(error) = pollster::block_on(instance(wgpu::Backends::empty()).request_adapter(&Default::default())) else {
            panic!("an empty backend set answered an adapter");
        };
        assert_eq!(reason, &format!("no adapter: {error}"));
        assert_eq!(gpu.summary(), reason.clone());
    }

    /// A device request that fails answers the reason too, through the production `open`: the
    /// adapter is real, the limits ask one above what it gives, and the answer is words the CPU
    /// renders with, not a panic: the adapter's name and wgpu's own refusal.
    #[test]
    fn failed_device_request_answers_a_reason() {
        let instance = instance(Platform::Host.backends());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            panic!("the host must have a non-GL adapter for this test to mean anything");
        };
        let mut too_much = required_limits(&adapter.limits());
        // One above what the adapter gives. A physical device's texture dimension sits far under
        // a `u32` overflow, and if one ever did not, the next assertion would say so.
        too_much.max_texture_dimension_2d = adapter.limits().max_texture_dimension_2d + 1;
        let Err(refusal) = pollster::block_on(adapter.request_device(&device_descriptor(too_much.clone()))) else {
            panic!("limits above the adapter's must not build a device");
        };
        let name = adapter.get_info().name;
        let gpu = open(&instance, adapter, too_much);
        let GpuDevice::CpuOnly { reason } = &gpu else {
            panic!("limits above the adapter's must not build a device");
        };
        assert_eq!(reason, &format!("no device on {name}: {refusal}"));
        assert_eq!(gpu.summary(), reason.clone());
    }

    /// GL is never requested, on any platform (§17); each platform's one-word pick is pinned so a
    /// mutation to any arm — or a dropped cap — fails here on the host.
    #[test]
    fn backends_are_metal_or_vulkan_and_never_gl() {
        assert_eq!(Platform::Ios.backends(), wgpu::Backends::METAL);
        assert_eq!(Platform::Android.backends(), wgpu::Backends::VULKAN);
        assert_eq!(Platform::Host.backends(), wgpu::Backends::all().difference(wgpu::Backends::GL));
        for platform in [Platform::Ios, Platform::Android, Platform::Host] {
            assert!(!platform.backends().contains(wgpu::Backends::GL), "{platform:?} requested GL");
        }
    }

    /// Each target the gate builds takes its own platform's pick: the phones by their
    /// `target_os` names (the iOS device and Simulator are both `ios`, the two Android ABIs
    /// `android`), and this host's build by its own name, so a swapped arm fails here on the host.
    #[test]
    fn each_target_takes_its_platform() {
        assert_eq!(Platform::of("ios"), Platform::Ios);
        assert_eq!(Platform::of("android"), Platform::Android);
        assert_eq!(Platform::of("macos"), Platform::Host);
        assert_eq!(Platform::current(), Platform::of(std::env::consts::OS));
    }

    /// The environment must not switch that pick (§17, D-7): the instance is built without reading it.
    /// Only a fresh process proves this — setting `WGPU_BACKEND` here would switch the adapter every
    /// other test asks for, and wgpu reads the variable as an instance is built, so it must already be
    /// in the environment when the production request runs. Were the instance ever built from the
    /// environment, the child would get GL — or, with GL not compiled in, no device at all, which is
    /// the same failure to pick — and this test would fail with the child's own words.
    #[test]
    fn a_child_started_with_wgpu_backend_gl_still_gets_the_core_s_backend() {
        craftbridge_process::rerun(
            "gpu::tests::child_asked_with_wgpu_backend_gl",
            &[("WGPU_BACKEND", std::ffi::OsStr::new("gl"))],
            "WGPU_BACKEND=gl switched the backend the core asks for",
        )
        .unwrap();
    }

    /// The production request as the parent above runs it in a child whose environment says
    /// `WGPU_BACKEND=gl`. The host answers a device on its own backend whatever the variable says: a
    /// GL device is the environment having switched the core's pick, and no device at all means the
    /// variable emptied the backends the core asked for.
    #[test]
    #[ignore = "run by a_child_started_with_wgpu_backend_gl_still_gets_the_core_s_backend, which starts it with WGPU_BACKEND=gl"]
    fn child_asked_with_wgpu_backend_gl() {
        let gpu = GpuDevice::request(GpuChoice::Preferred);
        let Some((adapter, _device, _queue)) = gpu.parts() else {
            panic!("the host answers a device whatever WGPU_BACKEND says, and this child answered none: {}", gpu.summary());
        };
        assert_ne!(adapter.get_info().backend, wgpu::Backend::Gl, "the core's instance read WGPU_BACKEND after all: {}", gpu.summary());
    }

    /// With a device, `info.gpu` reads `"<adapter> (<backend>)"` (§17's GPU-attached state), the
    /// backend spelt as the compositor names its device: `Metal`, `Vulkan`.
    #[test]
    fn summary_names_the_adapter_and_backend() {
        let gpu = GpuDevice::request(GpuChoice::Preferred);
        let GpuDevice::Device { adapter, .. } = &gpu else {
            panic!("a desktop with no adapter: {}", gpu.summary());
        };
        let info = adapter.get_info();
        let backend = match info.backend {
            wgpu::Backend::Metal => "Metal",
            wgpu::Backend::Vulkan => "Vulkan",
            wgpu::Backend::Dx12 => "Dx12",
            other => panic!("the host answered a backend a desktop never asks: {other:?}"),
        };
        assert_eq!(gpu.summary(), format!("{} ({backend})", info.name));
    }

    /// A device error raised while `validated` runs is the `Gpu` error in the caller's words and the
    /// device's, where with no scope open wgpu would have panicked; work that raises nothing is answered.
    #[test]
    fn a_device_error_inside_validated_is_a_gpu_error_not_a_panic() {
        let device = crate::testing::rig().device;
        let make = |width| crate::testing::texture(&device, (width, 4), 1, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::TEXTURE_BINDING);
        // A texture with no width is a validation error.
        let Err(CraftError::Gpu { message }) = validated(&device, "making a texture", || make(0)) else {
            panic!("a texture of no width must answer a Gpu error");
        };
        assert!(message.starts_with("making a texture: "), "{message}");
        assert!(validated(&device, "making a texture", || make(4)).is_ok());
    }

    /// `validated` opens a scope for every kind of error a device reports: one it left out would reach
    /// wgpu's default handler, which panics. The match names every kind, so a kind wgpu adds stops
    /// this compiling until it is weighed here.
    #[test]
    fn every_kind_of_device_error_is_captured() {
        for filter in [wgpu::ErrorFilter::Validation, wgpu::ErrorFilter::OutOfMemory, wgpu::ErrorFilter::Internal] {
            match filter {
                wgpu::ErrorFilter::Validation | wgpu::ErrorFilter::OutOfMemory | wgpu::ErrorFilter::Internal => {}
            }
            assert!(CAPTURED.contains(&filter), "{filter:?} reaches wgpu's panicking default handler");
        }
    }

    /// The engine hands a `GpuDevice` to its thread, so it must move between threads.
    #[test]
    fn gpu_device_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<GpuDevice>();
        assert_send::<GpuChoice>();
    }

    /// The failure notice keeps the first report's words and nothing after it: a failing device
    /// can report again, and only the first failure names why the render went back to the CPU.
    /// Every holder of the device shares the one notice — a second ask for it sees the same
    /// failure — so this is what is read to drop the GPU (§17). The notice must
    /// also cross threads: wgpu calls the callbacks that report into it on its own.
    #[test]
    fn device_failure_latches_on_the_first_report() {
        let gpu = GpuDevice::request(GpuChoice::Preferred);
        let GpuDevice::Device { .. } = &gpu else {
            panic!("a desktop with no adapter: {}", gpu.summary());
        };
        let note = gpu.device_failure_notifier();
        assert!(!note.failed());
        assert!(note.report("device lost: the first".to_string()), "the first report takes");
        assert!(!note.report("device lost: a later one".to_string()), "a later report changes nothing");
        assert!(note.failed());
        assert_eq!(note.reason().as_deref(), Some("device lost: the first"));
        assert!(gpu.device_failure_notifier().failed(), "every holder shares the one notice");
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GpuFailure>();
    }

    /// With no device there is nothing to fail: the notice a `CpuOnly` answer hands out never
    /// fails, and `info.gpu` keeps reporting the reason it already carries (§17).
    #[test]
    fn cpu_only_has_no_failure_to_report() {
        let gpu = GpuDevice::CpuOnly { reason: "the app asked for CPU-only rendering".to_string() };
        let note = gpu.device_failure_notifier();
        assert!(!note.failed());
        assert!(note.reason().is_none());
    }
}
