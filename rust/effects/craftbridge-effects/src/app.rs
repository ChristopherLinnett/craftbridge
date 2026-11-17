//! [`EffectCraftApp`]: the construction of plan §19 and the dispatch of the §14 method table.

use std::sync::Arc;

use craftbridge_core::folders::Folders;
use craftbridge_core::{CraftApp, CraftError, FrameBytes, GpuDevice, GpuFailure, PixelFormat, Platform, Result};
// The core and the engine both call their type `Confinement`: the core's carries the roots the app's
// folders make, the engine's checks every access against those same roots. They meet in this file
// alone, so the engine's is renamed here to keep the two apart.
use effectcraft_automation::confine::Confinement as EngineConfinement;
use effectcraft_automation::{Backend, McpServer};
use serde_json::Value;
use std::path::PathBuf;

use crate::runner::{self, ListOutputs, Progress};
use crate::{files, info, mcp, methods, plugins, properties, replay, session, templates, timings};

/// A hosted EffectCraft session: the MCP server built at construction, which owns the headless
/// [`Backend`] and the confined session behind it (plan §19).
///
/// The core moves the app onto the engine thread and reaches it only through its [`CraftApp`]
/// calls, so nothing here needs a lock: one call at a time is what the engine thread already gives
/// it.
pub struct EffectCraftApp {
    /// Every method reaches the session through this server's backend, so the app keeps the server
    /// itself rather than the backend inside it: the `mcp` call answers its lines from here, and an
    /// app that wired the line transport around a copy of the backend would fork the session.
    server: McpServer,
    /// The notice of the device last handed to [`CraftApp::attach_gpu`], kept so [`Self::drop_gpu`]
    /// can name in `info.gpu` the failure that ended it (§17). The engine reads the same notice and
    /// calls `drop_gpu` between two calls once it has a report; the app never polls it.
    gpu_failure: Option<GpuFailure>,
    /// The app's cache folder, inside the container, where a source the session may not write beside
    /// is copied to be read ([`crate::staging`]).
    cache: PathBuf,
    /// The app's place in the iOS plug-in gate's folder list, held while the app lives (§19).
    _admission: plugins::Admission,
}

impl EffectCraftApp {
    /// Build the app from the folders the app named (plan §19, steps 1 to 4): prepare the folders
    /// and make the confinement they derive, build the session, run it headless and confined, and
    /// serve it through the MCP server every method reaches it by.
    ///
    /// The folders are prepared first because the engine's confinement resolves its roots on disk,
    /// and a root that is not there yet would be refused before the folder that would be it exists.
    pub fn new(folders: &Folders) -> Result<Self> {
        Self::for_platform(Platform::current(), folders)
    }

    /// The app as `platform` builds it: [`Self::new`] for this build's own platform, and for the
    /// tests the same construction with another platform's plug-in policy (§19), so the policy of
    /// each platform is pinned on the host through the app's own door and confinement.
    pub(crate) fn for_platform(platform: Platform, folders: &Folders) -> Result<Self> {
        let roots = folders.prepare()?;
        // The iOS plug-in gate reads the app's plug-in folder from where the loader function can
        // reach it, since a loader is a plain function pointer (§19); the folder is resolved on
        // disk, so it is handed over once the folders are prepared.
        let admission = plugins::admit(platform, folders);
        let mut backend = Backend::headless(session::build(platform, folders));
        let confinement = EngineConfinement::new(roots.read_roots(), &[roots.write_root().to_path_buf()]).map_err(|error| CraftError::io(error.to_string()))?;
        backend.confine(confinement);
        Ok(Self { server: McpServer::new(backend), gpu_failure: None, cache: folders.cache.clone(), _admission: admission })
    }

    /// The parameters of a call this app answers, with a template the session may not write beside
    /// replaced by a copy inside the container ([`templates::staged`]): the one place every call that
    /// names a template passes through, once it is known to answer the method.
    fn staged(&mut self, method: &str, params: Value) -> Result<Value> {
        templates::staged(self.server.backend(), &self.cache, method, params)
    }
}

/// One method's answer: the backend the session sits behind, and the parameters the call carried.
type Method = fn(&mut Backend, &Value) -> Result<Value>;

/// The methods of the plan's §14 table `request` answers, by name. It is the one list of them: the
/// dispatch reads it, and the refusal of every other name recites it, so the refusal names what
/// `request` answers truly whatever a build adds to it.
const METHODS: [(&str, Method); 14] = [
    ("info", info::info),
    ("commands", methods::commands),
    ("exec", methods::exec),
    ("run", methods::run),
    ("props", properties::props),
    ("get", properties::get),
    ("set", properties::set),
    ("open", files::open),
    ("save", files::save),
    ("saveAs", files::save_as),
    ("new", files::new),
    ("replay", replay::replay),
    ("timings", timings::timings),
    ("template.open", templates::open),
];

/// One method that runs long enough to report progress: the backend, the parameters, the listener
/// that answers `false` to stop it, and the lister of the files it may write.
type ProgressMethod = for<'a> fn(&mut Backend, &Value, Progress<'a>, ListOutputs<'a>) -> Result<Value>;

/// The methods `request_with_progress` answers, by name, the one list of them as [`METHODS`] is of
/// `request`'s: the dispatch reads it and so does its refusal.
const PROGRESS_METHODS: [(&str, ProgressMethod); 2] = [("export", runner::export), ("template.render", templates::render)];

impl CraftApp for EffectCraftApp {
    /// Answer one method of the plan's §14 table through the function [`METHODS`] names for it.
    /// Every other name — a §14 method `request` does not answer, or no method at all — answers
    /// `Unsupported`, the row of a capability this build lacks, naming what was asked and the
    /// methods `request` does answer: a newer app asking an older build for a method it lacks is
    /// the case the refusal is for. It speaks of `request` alone because the core reaches the app
    /// through calls of its own for the binary `frame`, the line `mcp` and a call with progress
    /// (plan §16), so a name refused here can be one the app answers through another call.
    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        match METHODS.iter().find(|(name, _)| *name == method) {
            Some((_, answer)) => {
                let params = self.staged(method, params)?;
                answer(self.server.backend(), &params)
            }
            None => Err(CraftError::unsupported(not_answered(method, "request", METHODS.iter().map(|(name, _)| *name)))),
        }
    }

    /// Answer one method of the plan's §14 table that reports progress through the function
    /// [`PROGRESS_METHODS`] names for it. Every other name answers `Unsupported`, as the core's
    /// default for an app with no such call does, naming the methods this call does answer. The
    /// method lists the files it may write through `list_outputs` before it writes any.
    fn request_with_progress(
        &mut self,
        method: &str,
        params: Value,
        on_progress: &mut dyn FnMut(u64, u64, Option<f64>) -> bool,
        list_outputs: &mut dyn FnMut(&[PathBuf]) -> Result<()>,
    ) -> Result<Value> {
        match PROGRESS_METHODS.iter().find(|(name, _)| *name == method) {
            Some((_, answer)) => {
                let params = self.staged(method, params)?;
                answer(self.server.backend(), &params, on_progress, list_outputs)
            }
            None => Err(CraftError::unsupported(not_answered(method, "request_with_progress", PROGRESS_METHODS.iter().map(|(name, _)| *name)))),
        }
    }

    /// Render one frame (plan §14's binary `frame`): the request is resolved as `render-frame`
    /// resolves its flags, and when it names a `template` the frame is that template's instance, as
    /// `render-frame --template` renders it. It has no row in [`METHODS`], so `request` keeps refusing
    /// the name.
    fn frame(&mut self, params: Value, format: PixelFormat) -> Result<FrameBytes> {
        let params = self.staged(templates::FRAME, params)?;
        templates::frame(self.server.backend(), &params, format)
    }

    /// Answer one line of the MCP transport through the server built at construction, which owns the
    /// session every method reaches. It is not a row of [`METHODS`]: a line is not a method call, so
    /// `request` keeps refusing the name `mcp`.
    fn mcp(&mut self, line: &str) -> Option<String> {
        mcp::answer(&mut self.server, line)
    }

    /// Take the device the core built (§17). A device is handed to EffectCraft's own
    /// `Gpu::new(adapter, device, queue)` — the hand-over EffectCraft's desktop app makes of its
    /// window's device — and the session renders on the result: `Session.accel` carries it, the CPU
    /// note goes away, and `info.gpu` names the adapter (§19 step 5). A `CpuOnly` answer is not a
    /// failure: its reason becomes the session's note, and every render runs on the CPU (L7).
    ///
    /// Both of wgpu's failure callbacks are then installed on the device — the device-lost callback
    /// wgpu calls when the driver retires it, and the uncaptured-error callback wgpu calls for an
    /// error no call claimed — and both call one failure path, as the compositor's own installer
    /// does (`crates/gpu/src/readback.rs`:348-362), in the words it and EffectCraft's desktop app
    /// give each. That path reports to the core's notice for this device and to the compositor's
    /// shared-device notifier, which a borrowed device's owner must forward to so pending readbacks
    /// settle the moment the device is gone (`crates/gpu/src/context.rs`:826-830, :1070-1078). The
    /// first report is the one the notice keeps, and the engine drops the GPU before the next call
    /// (§17).
    fn attach_gpu(&mut self, gpu: &GpuDevice) -> Result<()> {
        let Some((adapter, device, queue)) = gpu.parts() else {
            let Some(session) = self.server.backend().session() else {
                return Err(CraftError::gpu("this app's session sits behind a bridge, which holds no GPU to note"));
            };
            session.accel = None;
            session.accel_note = Some(gpu.summary());
            self.gpu_failure = None;
            return Ok(());
        };
        let accel = effectcraft_gpu::Gpu::new(adapter, device.clone(), queue.clone()).map_err(CraftError::gpu)?;
        let note = gpu.device_failure_notifier();
        let fail = {
            let (note, retire) = (note.clone(), accel.context().device_failure_notifier());
            Arc::new(move |why: String| {
                retire(why.clone());
                note.report(why);
            })
        };
        let lost = Arc::clone(&fail);
        device.set_device_lost_callback(move |reason, message| lost(format!("GPU device lost ({reason:?}): {message}")));
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| fail(format!("uncaptured GPU error: {error}"))));
        let Some(session) = self.server.backend().session() else {
            return Err(CraftError::gpu("this app's session sits behind a bridge, which holds no GPU to attach"));
        };
        session.accel = Some(Arc::new(accel));
        session.accel_note = None;
        self.gpu_failure = Some(note);
        Ok(())
    }

    /// Put the session back on the CPU: the device goes out of the session, and the note takes the
    /// first failure's words when a failure is why (§17), as EffectCraft's desktop app notes a
    /// retired device (`crates/ui-egui/src/app/gpu.rs`:69-70), so `info.gpu` names it afterwards.
    /// Nothing was attached: nothing to drop — the app keeps answering what it answered before.
    fn drop_gpu(&mut self) {
        let Some(session) = self.server.backend().session() else {
            return;
        };
        if session.accel.is_none() {
            return;
        }
        session.accel = None;
        session.accel_note = Some(match self.gpu_failure.as_ref().and_then(GpuFailure::reason) {
            Some(why) => format!("GPU preview retired: {why}"),
            None => "the GPU was detached; renders run on the CPU".to_string(),
        });
    }
}

/// The refusal of a name one entry point does not answer: true of any name in any build, since it says
/// only that the entry point lacks it and which methods it has, read from its table.
fn not_answered<'a>(method: &str, entry: &str, answered: impl Iterator<Item = &'a str>) -> String {
    let names: Vec<String> = answered.map(|name| format!("`{name}`")).collect();
    let answered = match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        Some((last, _)) => last.clone(),
        None => "nothing".to_string(),
    };
    format!("`{method}` is not answered by `{entry}` in this build, which answers {answered}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::tests::Scratch;
    use craftbridge_core::GpuChoice;

    /// The failure path tells the compositor as well as the core, whichever of wgpu's two callbacks
    /// reports: after the failure, and before any request has dropped the GPU, the compositor's own
    /// context on that device is already retired (`Accelerator::available`, which is its health
    /// check), so its pending readbacks settle at once rather than waiting out their deadlines
    /// (§17).
    #[test]
    fn a_failure_retires_the_compositor_before_the_gpu_is_dropped() {
        let causes: [(&str, fn(&wgpu::Device)); 2] = [
            ("an uncaptured error", |device| {
                let oversized = wgpu::BufferDescriptor {
                    label: Some("craftbridge compositor-retire probe"),
                    size: device.limits().max_buffer_size + 4096,
                    usage: wgpu::BufferUsages::UNIFORM,
                    mapped_at_creation: false,
                };
                let _ = device.create_buffer(&oversized);
            }),
            ("a lost device", |device| {
                device.destroy();
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
            }),
        ];
        for (name, cause) in causes {
            let scratch = Scratch::new(false, None);
            let mut app = scratch.app(Platform::Host);
            let gpu = GpuDevice::request(GpuChoice::Preferred);
            let GpuDevice::Device { device, .. } = &gpu else {
                panic!("a desktop with no adapter: {}", gpu.summary());
            };
            app.attach_gpu(&gpu).expect("the desktop device attaches");
            let available = |app: &mut EffectCraftApp| app.server.backend().session().and_then(|session| session.accel.as_ref().map(|accel| accel.available()));
            assert_eq!(available(&mut app), Some(true), "the compositor starts healthy");
            cause(device);
            assert_eq!(available(&mut app), Some(false), "{name}: the failure path retired the compositor's context");
        }
    }
}
