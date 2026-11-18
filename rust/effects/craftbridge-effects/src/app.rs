//! [`EffectCraftApp`]: the construction of plan §19 and the dispatch of the §14 method table.

use std::sync::Arc;

use craftbridge_core::folders::Folders;
use craftbridge_core::{CraftApp, CraftError, FrameBytes, GpuDevice, GpuFailure, PixelFormat, Platform, Result, Surface};
// The core and the engine both call their type `Confinement`: the core's carries the roots the app's
// folders make, the engine's checks every access against those same roots. They meet in this file
// alone, so the engine's is renamed here to keep the two apart.
use effectcraft_automation::confine::Confinement as EngineConfinement;
use effectcraft_automation::{Backend, McpServer};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::hardware::HardwareChoice;
use crate::requests::{self, RenderRequest};
use crate::runner::{self, ListOutputs, Progress};
use crate::{files, fill, info, mcp, methods, orphans, plugins, properties, replay, session, templates, timings};

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
    /// The compositor on the device last handed to [`CraftApp::attach_gpu`]: what `present` leaves a
    /// display frame on, which the preview surface blits without a copy through the CPU (§17). It is the
    /// same device the session's `accel` renders on, a cheap clone of it, and goes where that goes.
    display: Option<effectcraft_gpu::Gpu>,
    /// The app's staged folder, inside the container, where a source the session may not write beside
    /// is copied to be read ([`crate::staging`]). Not the cache: a saved project names these copies,
    /// and the system may empty the cache while the app is not running.
    staged: PathBuf,
    /// Whether the export being answered chose hardware encoding, which the session's exporter reads.
    hardware: HardwareChoice,
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
        // A long write the system killed leaves its temporary file beside the file it was writing,
        // and no journal could list that file: its name is drawn as the write starts, later than
        // any listing of it could be. The start takes those back here, in the one path an engine
        // of this app starts by — the core deals with the files the pending-export journal still
        // lists one step on, in `Engine::start_in` — and takes nothing else (AD21).
        orphans::sweep(folders);
        // The iOS plug-in gate reads the app's plug-in folder from where the loader function can
        // reach it, since a loader is a plain function pointer (§19); the folder is resolved on
        // disk, so it is handed over once the folders are prepared.
        let admission = plugins::admit(platform, folders);
        // The session's exporter reads this choice while an export is being answered; the app starts
        // it with nothing chosen, and `request_with_progress` sets it for the length of one call.
        let hardware = HardwareChoice::default();
        let mut backend = Backend::headless(session::build(platform, folders, &hardware));
        let confinement = EngineConfinement::new(roots.read_roots(), &[roots.write_root().to_path_buf()]).map_err(|error| CraftError::io(error.to_string()))?;
        backend.confine(confinement);
        Ok(Self { server: McpServer::new(backend), gpu_failure: None, display: None, staged: folders.staged.clone(), hardware, _admission: admission })
    }

    /// The parameters of a call this app answers, with a template the session may not write beside
    /// replaced by a copy inside the container ([`templates::staged`]): the one place every call that
    /// names a template passes through, once it is known to answer the method.
    fn staged(&mut self, method: &str, params: Value) -> Result<Value> {
        templates::staged(self.server.backend(), &self.staged, method, params)
    }
}

/// One method's answer, by what it is handed. Every method is handed the backend the session sits
/// behind and the parameters the call carried; a method that runs the commands the app names is handed
/// the app's staged folder too, where the door puts the copy a command that writes beside its source
/// reads ([`crate::imports`]). Each row of [`METHODS`] says which kind its method is, so the folder
/// reaches a method as a parameter of the call answering it, and by no other way.
enum Method {
    /// A method that runs only commands of the plugin's own choosing, none of which the door copies for.
    Own(fn(&mut Backend, &Value) -> Result<Value>),
    /// A method that runs the commands the app names.
    Commands(fn(&mut Backend, &Path, &Value) -> Result<Value>),
}

/// The methods of the plan's §14 table `request` answers, by name. It is the one list of them: the
/// dispatch reads it, and the refusal of every other name recites it, so the refusal names what
/// `request` answers truly whatever a build adds to it.
const METHODS: [(&str, Method); 14] = [
    ("info", Method::Own(info::info)),
    ("commands", Method::Own(methods::commands)),
    ("exec", Method::Commands(methods::exec)),
    ("run", Method::Commands(methods::run)),
    ("props", Method::Own(properties::props)),
    ("get", Method::Own(properties::get)),
    ("set", Method::Own(properties::set)),
    ("open", Method::Own(files::open)),
    ("save", Method::Own(files::save)),
    ("saveAs", Method::Own(files::save_as)),
    ("new", Method::Own(files::new)),
    ("replay", Method::Commands(replay::replay)),
    ("timings", Method::Own(timings::timings)),
    ("template.open", Method::Own(templates::open)),
];

/// One method that runs long enough to report progress: the backend, the parameters, the listener
/// that answers `false` to stop it, and the lister of the files it may write.
type ProgressMethod = for<'a> fn(&mut Backend, &Value, Progress<'a>, ListOutputs<'a>) -> Result<Value>;

/// Where a progress method's render request sits in its parameters, which is what the session's hardware
/// choice is read from.
type RequestOf = fn(&Value) -> Option<&Value>;

/// The methods `request_with_progress` answers, by name, the one list of them as [`METHODS`] is of
/// `request`'s: the dispatch reads it and so does its refusal. Each names where its render request sits,
/// so a method added here cannot render without its hardware choice being read.
const PROGRESS_METHODS: [(&str, ProgressMethod, RequestOf); 3] = [
    ("export", runner::export, runner::export_request),
    ("template.render", templates::render, templates::render_request),
    (methods::FILL, fill::generate, fill::render_request),
];

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
                match answer {
                    Method::Own(answer) => answer(self.server.backend(), &params),
                    Method::Commands(answer) => answer(self.server.backend(), &self.staged, &params),
                }
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
        match PROGRESS_METHODS.iter().find(|(name, ..)| *name == method) {
            Some((_, answer, request_of)) => {
                let params = self.staged(method, params)?;
                let _chosen = self.hardware.scope(request_of(&params).is_some_and(RenderRequest::chooses_hardware));
                answer(self.server.backend(), &params, on_progress, list_outputs)
            }
            None => Err(CraftError::unsupported(not_answered(method, "request_with_progress", PROGRESS_METHODS.iter().map(|(name, ..)| *name)))),
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

    /// Draw one preview frame (plan §14's `present`): the request is a `frame` call's, resolved as a
    /// `frame` resolves it, and the frame is drawn into the surface the core hands over, on the GPU
    /// where the app holds one and the project asks for GPU acceleration, and from the CPU's pixels
    /// otherwise, the same picture either way ([`requests::picture`], §17).
    fn present(&mut self, params: Value, surface: &mut Surface) -> Result<()> {
        requests::present(self.server.backend(), self.display.as_ref(), &params, surface)
    }

    /// Answer one line of the MCP transport through the server built at construction, which owns the
    /// session every method reaches. It is not a row of [`METHODS`]: a line is not a method call, so
    /// `request` keeps refusing the name `mcp`.
    fn mcp(&mut self, line: &str) -> Option<String> {
        mcp::answer(&mut self.server, &self.staged, line)
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
            self.display = None;
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
        self.display = Some(accel.clone());
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
        self.display = None;
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
    use serde_json::json;

    /// Every progress method's render request is found where the method reads it, so the hardware choice
    /// is read from the request the method renders and from nothing else in its parameters.
    #[test]
    fn each_progress_method_names_where_its_render_request_sits() {
        let request = json!({"output": "/c/x.mp4", "hardware": true});
        let calls = [("export", request.clone()), ("template.render", json!({"template": "/c/t.ectemplate", "output": request.clone(), "hardware": false}))];
        for (method, params) in calls {
            let (_, _, request_of) = PROGRESS_METHODS.iter().find(|(name, ..)| *name == method).expect("a progress method");
            assert_eq!(request_of(&params), Some(&request), "{method}");
        }
        let (_, _, request_of) = PROGRESS_METHODS.iter().find(|(name, ..)| *name == "template.render").expect("a progress method");
        assert_eq!(request_of(&json!({"template": "/c/t.ectemplate"})), None, "a render without its request chooses nothing");
        let (_, _, request_of) = PROGRESS_METHODS.iter().find(|(name, ..)| *name == methods::FILL).expect("a progress method");
        assert_eq!(request_of(&json!({"layer": 1, "outputDir": "/c/fill"})), None, "a fill renders no movie, so it holds no render request");
    }

    /// The app as the host builds it, handed the desktop's own device, and that device.
    fn on_the_desktop_gpu(scratch: &Scratch) -> (EffectCraftApp, GpuDevice) {
        let mut app = scratch.app(Platform::Host);
        let gpu = GpuDevice::request(GpuChoice::Preferred);
        assert!(gpu.parts().is_some(), "a desktop with no adapter: {}", gpu.summary());
        app.attach_gpu(&gpu).expect("the desktop device attaches");
        (app, gpu)
    }

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
            let (mut app, gpu) = on_the_desktop_gpu(&scratch);
            let (_, device, _) = gpu.parts().expect("the desktop device");
            let available = |app: &mut EffectCraftApp| app.server.backend().session().and_then(|session| session.accel.as_ref().map(|accel| accel.available()));
            assert_eq!(available(&mut app), Some(true), "the compositor starts healthy");
            cause(device);
            assert_eq!(available(&mut app), Some(false), "{name}: the failure path retired the compositor's context");
        }
    }

    /// A preview is one picture whichever compositor draws it: the compositor's display frame while the
    /// project asks for GPU acceleration, and the CPU's frame once it is on Mercury Software Only, as its
    /// frames render, and the two hold the same premultiplied pixels — the comp's own alpha, never its
    /// background colour, which the display frame does not carry (plan §17).
    #[test]
    fn a_preview_is_one_picture_on_the_gpu_and_on_the_cpu_and_follows_the_projects_renderer() {
        let scratch = Scratch::new(false, None);
        let (mut app, gpu) = on_the_desktop_gpu(&scratch);
        let max_side = gpu.parts().expect("the desktop device").1.limits().max_texture_dimension_2d;
        let steps = json!({"steps": [
            {"command": "comp.new", "params": {"name": "C", "width": 8, "height": 6, "background": "#ffffff"}},
            {"command": "layer.newSolid", "params": {"comp": 1, "name": "Box", "color": "#ff8040", "width": 4, "height": 2}},
            {"command": "prop.set", "params": {"layer": "Box", "path": "transform/opacity", "value": 50}}
        ]});
        app.request("run", steps).expect("the comp is made");
        let picture = |app: &mut EffectCraftApp| requests::picture(app.server.backend(), app.display.as_ref(), &json!({}), max_side).expect("a picture");

        let requests::Picture::Display(display) = picture(&mut app) else {
            panic!("a project on GPU acceleration previews the compositor's display frame");
        };
        let on_gpu = app.display.as_ref().and_then(|gpu| gpu.read_display(&display)).expect("the display frame reads back");

        app.request("run", json!({"steps": [{"command": "file.projectSettings", "params": {"renderer": "software"}}]})).expect("the project goes to the CPU");
        let requests::Picture::Pixels(frame) = picture(&mut app) else {
            panic!("a project on Mercury Software Only previews from the CPU, as its frames render");
        };
        assert_eq!((frame.width, frame.height), (display.width, display.height), "the same size of picture");
        // The core's own conversion, which the preview's upload makes of the straight pixels, red and blue
        // exchanged back to the display frame's order.
        let premultiplied = FrameBytes::new(frame.width, frame.height, PixelFormat::Bgra8Premultiplied, frame.data).expect("the frame converts");
        let on_cpu: Vec<u8> = premultiplied.data.as_chunks::<4>().0.iter().flat_map(|&[blue, green, red, alpha]| [red, green, blue, alpha]).collect();
        assert_eq!(on_cpu.get(..4), Some(&[0, 0, 0, 0][..]), "a corner the box leaves is transparent, not the comp's white background");
        assert_eq!(on_gpu.len(), on_cpu.len());
        let apart = on_gpu.iter().zip(&on_cpu).map(|(gpu, cpu)| gpu.abs_diff(*cpu)).max();
        assert!(apart <= Some(1), "the GPU's and the CPU's pictures differ by {apart:?}: {on_gpu:?} against {on_cpu:?}");
    }
}
