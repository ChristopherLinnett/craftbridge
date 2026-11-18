//! The engine's GPU lifecycle (plan §14, §16, §17, L7): the device the engine asked for at its
//! start, dropped on the first failure its notice reports, and given up and asked for again as the
//! app goes to the background and comes back.
//!
//! It lives on the engine thread beside the session, and is reached only from the engine's loop, so
//! every change of GPU happens between two calls and never inside one. That is what lets the
//! app's `attach_gpu` and `drop_gpu` assume no render is running.

use crate::Result;
use crate::app::CraftApp;
use crate::engine::guarded;
use crate::error::CraftError;
use crate::gpu::{GpuChoice, GpuDevice};
use crate::preview::{NO_DEVICE, Surface, SurfaceMaker};
use serde_json::Value;

/// What the engine holds of the GPU: the device it handed the app, and the preview surface made on
/// that device. Neither is held with no device: a surface outlives its device by nothing but a
/// background submission, which iOS ends an app for.
pub(crate) struct GpuLifecycle {
    /// What the app asked for when it started the engine, which every later request for a device
    /// asks again, so an engine that chose `CpuOnly` never gets one by resuming (plan §14).
    choice: GpuChoice,
    /// Declared before `held`, so the surface is dropped before the device it was configured on.
    preview: Option<Surface>,
    /// The device the app was handed, kept for its failure notice: `Some` only while the app holds
    /// a real device, so the engine is on the GPU exactly when this is.
    held: Option<GpuDevice>,
}

impl GpuLifecycle {
    /// Ask for the device `choice` allows and hand it to `app`. The answer a start wants is an
    /// engine, so a device that cannot be had, or an app that cannot take it, leaves the engine on
    /// the CPU with `info.gpu` saying why: no GPU with `Preferred` is not a failure to start (plan
    /// §16).
    pub(crate) fn start(app: &mut dyn CraftApp, choice: GpuChoice) -> Self {
        let mut lifecycle = Self { choice, preview: None, held: None };
        if let Err(refused) = guarded(|| lifecycle.attach(app)).flatten() {
            // A start has no caller to throw the refusal to, so the app is handed it as its CPU note
            // and `info.gpu` names it, rather than a note that says nothing of why.
            let note = GpuDevice::CpuOnly { reason: format!("the app could not take the GPU: {refused}") };
            let _ = guarded(|| app.attach_gpu(&note));
        }
        lifecycle
    }

    /// Make the preview surface `make` builds on the device the app was handed, in place of the one
    /// attached: that one is given up first, since Vulkan makes one surface of an Android window at
    /// a time and an app may attach the window it already attached, so an attach that fails leaves
    /// no surface and every present answers so until one succeeds. With no device, whether the engine
    /// chose the CPU, found no adapter or gave its device up for the background, there is nothing to
    /// preview on and `make` never runs: the app attaches again once it holds a device.
    pub(crate) fn attach_preview(&mut self, make: SurfaceMaker) -> Result<()> {
        self.preview = None;
        let device = self.held.as_ref().ok_or_else(|| CraftError::gpu(NO_DEVICE))?;
        self.preview = Some(make(device)?);
        Ok(())
    }

    /// Run one `present` call on the app: the app takes the frame for its `params` and draws it
    /// through the surface. With no surface — never attached, after an attach that failed, after
    /// `detach_preview` or with the GPU gone — the call answers `Gpu { "no surface" }` and the
    /// session is untouched (plan §17).
    pub(crate) fn present_preview(&mut self, app: &mut dyn CraftApp, params: Value) -> Result<()> {
        let surface = self.preview.as_mut().ok_or_else(|| CraftError::gpu("no surface"))?;
        app.present(params, surface)
    }

    /// Drop the preview surface, and the platform handle and frame upload it holds. A detach with
    /// nothing attached changes nothing.
    pub(crate) fn detach_preview(&mut self) {
        self.preview = None;
    }

    /// Whether a surface is attached to preview into: whether a `present` has something to draw on.
    /// The engine is the one holder of that answer — a device lost or a [`GpuLifecycle::suspend`]
    /// takes the surface with no notice to anyone else — so a preview that attaches only when it
    /// must asks here rather than keeping a copy of the fact of its own.
    pub(crate) const fn holds_preview(&self) -> bool {
        self.preview.is_some()
    }

    /// Drop the GPU if the notice of the device the app holds has a report: the first device-lost
    /// or uncaptured error, which the app's own callbacks wrote there. It runs before every call the
    /// engine makes on the app, so no call runs on a device that already failed, and the render
    /// goes to the CPU from then on (plan §17, L7). A panic in the app's `drop_gpu` is the app's:
    /// it is caught, the GPU is gone either way, and the call that follows runs on the session as
    /// the panic left it.
    pub(crate) fn settle(&mut self, app: &mut dyn CraftApp) {
        if self.held.as_ref().is_some_and(|device| device.device_failure_notifier().failed()) {
            let _ = guarded(|| self.release(app));
        }
    }

    /// Give up the GPU for the background: the preview surface, the app's device and the engine's
    /// own handle, so nothing of the engine holds a device that could take a submission. Every
    /// render answers on the CPU until [`GpuLifecycle::resume`]. Suspending twice is not an error.
    pub(crate) fn suspend(&mut self, app: &mut dyn CraftApp) {
        self.release(app);
    }

    /// Ask for a device again and hand it to `app`, as the start does, answering `None` once the app
    /// holds a device and the CPU note when none came. An engine that holds a device keeps it: a
    /// resume without a suspend, which an app that never left the foreground still makes, must not
    /// replace a working device with a second one. A request that answers no device is handed over
    /// all the same, as the CPU note `info.gpu` reports, and the engine stays on the CPU.
    ///
    /// # Errors
    ///
    /// What the app answers [`CraftApp::attach_gpu`] when it cannot take the device: the engine
    /// stays on the CPU, and a later resume asks again.
    pub(crate) fn resume(&mut self, app: &mut dyn CraftApp) -> Result<Option<String>> {
        if self.held.is_some() {
            return Ok(None);
        }
        self.attach(app)
    }

    /// Drop the surface, take the device out of the app, and let go of the engine's handle, in that
    /// order: a surface is configured on a device, and the handle goes last so the device lives
    /// until the app has taken its own out of the session. The handles are out of the engine's
    /// hands before the app's call runs, so a panic in it leaves nothing held.
    fn release(&mut self, app: &mut dyn CraftApp) {
        self.preview = None;
        let _held = self.held.take();
        app.drop_gpu();
    }

    /// Ask for a device and hand it over, answering the CPU note when no device came. The engine
    /// keeps the device only when the app took a real one, so a refusal leaves it on the CPU.
    fn attach(&mut self, app: &mut dyn CraftApp) -> Result<Option<String>> {
        // The request is the one place the core's own wgpu work runs; a panic in it is a device that
        // could not be had, not an engine that stops.
        let device =
            guarded(|| GpuDevice::request(self.choice)).unwrap_or_else(|error| GpuDevice::CpuOnly { reason: format!("the GPU request failed: {error}") });
        app.attach_gpu(&device)?;
        if device.parts().is_none() {
            return Ok(Some(device.summary()));
        }
        self.held = Some(device);
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cancel::CancelToken;
    use crate::engine::{Answer, Engine};
    use crate::error::CraftError;
    use crate::frames::{FrameBytes, PixelFormat};
    use crate::gpu::GpuFailure;
    use crate::surface::Swapchain;
    use crate::testing::{Gate, Scripted, rig};
    use serde_json::{Value, json};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// What the test and its probe app share: the calls the app saw in the order it saw them, the GPU
    /// work it submitted (a counting wrapper over its queue), the failure notice of each device it was
    /// handed, and the gates an export held by `hold_export` waits at.
    #[derive(Default)]
    struct Shared {
        events: Mutex<Vec<String>>,
        submissions: AtomicUsize,
        notices: Mutex<Vec<GpuFailure>>,
        devices: Mutex<Vec<wgpu::Device>>,
        keep_devices: AtomicBool,
        refuse: AtomicBool,
        panic_on_attach: AtomicBool,
        /// While set, an export stops after its first beat until the test opens `release`, and after
        /// its second until the test opens `finish`, so a test can hand the engine calls mid-export.
        hold_export: AtomicBool,
        /// Opened by the export once its first beat is done and it waits for `release`.
        held: Gate,
        release: Gate,
        /// Opened by the export once its second beat, which took every call made while it was held
        /// out of the inbox, is done and it waits for `finish`.
        drained: Gate,
        finish: Gate,
    }

    impl Shared {
        fn log(&self, event: String) {
            self.events.lock().unwrap().push(event);
        }

        fn events(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }

        fn notice(&self, attached: usize) -> GpuFailure {
            self.notices.lock().unwrap().get(attached).cloned().expect("the app was handed that many devices")
        }
    }

    /// An app that does what the EffectCraft app does with a device: takes its queue, installs both
    /// failure callbacks into the device's notice, and renders on it while it holds it, submitting
    /// one batch of GPU work per render and per beat of an export. It answers where it rendered.
    struct Probe {
        shared: Arc<Shared>,
        queue: Option<wgpu::Queue>,
    }

    impl Probe {
        fn new() -> (Probe, Arc<Shared>) {
            let shared = Arc::new(Shared::default());
            (Probe { shared: shared.clone(), queue: None }, shared)
        }

        /// Render once where the app is: on the GPU, counted, while it holds a queue.
        fn render(&self) -> Value {
            match &self.queue {
                Some(queue) => {
                    queue.submit([]);
                    self.shared.submissions.fetch_add(1, Ordering::SeqCst);
                    json!("gpu")
                }
                None => json!("cpu"),
            }
        }
    }

    impl CraftApp for Probe {
        fn request(&mut self, method: &str, _params: Value) -> Result<Value> {
            self.shared.log(format!("call {method}"));
            Ok(if method == "render" { self.render() } else { Value::Null })
        }

        fn request_with_progress(
            &mut self,
            method: &str,
            _params: Value,
            on_progress: &mut dyn FnMut(u64, u64, Option<f64>) -> bool,
            _list_outputs: &mut dyn FnMut(&[PathBuf]) -> Result<()>,
        ) -> Result<Value> {
            self.shared.log(format!("call {method} with progress"));
            let held = self.shared.hold_export.load(Ordering::SeqCst);
            let mut rendered = self.render();
            on_progress(1, 2, None);
            if held {
                self.shared.held.open();
                assert!(self.shared.release.wait(), "the test never released the export");
            }
            on_progress(2, 2, None);
            if held {
                rendered = self.render();
                self.shared.drained.open();
                assert!(self.shared.finish.wait(), "the test never let the export finish");
            }
            Ok(rendered)
        }

        fn frame(&mut self, _params: Value, format: PixelFormat) -> Result<FrameBytes> {
            self.shared.log("call frame".to_string());
            FrameBytes::new(1, 1, format, vec![0, 0, 0, 255])
        }

        fn mcp(&mut self, _line: &str) -> Option<String> {
            self.shared.log("call mcp".to_string());
            None
        }

        fn present(&mut self, _params: Value, surface: &mut Surface) -> Result<()> {
            self.shared.log("present".to_string());
            surface.present_frame(&FrameBytes::new(1, 1, PixelFormat::Rgba8Straight, vec![0, 0, 0, 255])?).map(drop)
        }

        fn attach_gpu(&mut self, gpu: &GpuDevice) -> Result<()> {
            self.shared.log(format!("attach {}", gpu.summary()));
            if self.shared.panic_on_attach.load(Ordering::SeqCst) {
                panic!("the app panicked taking the device");
            }
            // A real app refuses a device it cannot render on, never the note that it has none.
            if self.shared.refuse.load(Ordering::SeqCst) && gpu.parts().is_some() {
                return Err(CraftError::gpu("the app cannot take this device"));
            }
            let notice = gpu.device_failure_notifier();
            self.shared.notices.lock().unwrap().push(notice.clone());
            let Some((_, device, queue)) = gpu.parts() else { return Ok(()) };
            let lost_notice = notice.clone();
            device.set_device_lost_callback(move |reason, message| {
                lost_notice.report(format!("GPU device lost ({reason:?}): {message}"));
            });
            device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
                notice.report(format!("uncaptured GPU error: {error}"));
            }));
            if self.shared.keep_devices.load(Ordering::SeqCst) {
                self.shared.devices.lock().unwrap().push(device.clone());
            }
            self.queue = Some(queue.clone());
            Ok(())
        }

        fn drop_gpu(&mut self) {
            self.shared.log("drop_gpu".to_string());
            self.queue = None;
        }
    }

    fn rendered(engine: &Engine) -> Value {
        engine.request("render", json!({})).expect("the render answers")
    }

    fn exported(engine: &Engine) -> Result<Value> {
        engine.request_with_progress("export", json!({}), |_, _, _| {}, &CancelToken::new())
    }

    /// The adapter this machine's own request names: what `info.gpu` must say of an attached device.
    fn host_adapter() -> String {
        GpuDevice::request(GpuChoice::Preferred).summary()
    }

    fn attached_host() -> String {
        format!("attach {}", host_adapter())
    }

    /// What an engine on the host's device logs once one render ran on it: the app's start and that call.
    fn after_one_render() -> Vec<String> {
        vec![attached_host(), "call render".to_string()]
    }

    #[test]
    fn an_engine_started_with_a_gpu_hands_it_to_the_app_before_its_first_call() {
        let (app, shared) = Probe::new();
        let engine = Engine::start(app, GpuChoice::Preferred).expect("the engine starts");
        assert_eq!(rendered(&engine), json!("gpu"), "the first call already runs on the device");
        assert_eq!(shared.events(), after_one_render());
    }

    #[test]
    fn an_engine_that_asks_for_no_gpu_hands_the_app_the_cpu_note() {
        let (app, shared) = Probe::new();
        let engine = Engine::start(app, GpuChoice::CpuOnly).expect("the engine starts");
        assert_eq!(rendered(&engine), json!("cpu"));
        assert_eq!(shared.events(), ["attach the app asked for CPU-only rendering", "call render"]);
    }

    /// A start has no caller to throw to, so an app that refuses the device is handed the refusal as
    /// its CPU note, which `info.gpu` reports, and the engine answers on the CPU until a resume.
    #[test]
    fn an_app_that_cannot_take_the_device_starts_on_the_cpu_and_is_told_why() {
        let (app, shared) = Probe::new();
        shared.refuse.store(true, Ordering::SeqCst);
        let engine = Engine::start(app, GpuChoice::Preferred).expect("a refused device is no failure to start");
        assert_eq!(rendered(&engine), json!("cpu"));
        let note = "attach the app could not take the GPU: the app cannot take this device".to_string();
        assert_eq!(shared.events(), [attached_host(), note, "call render".into()], "the refusal is the app's CPU note");
        shared.refuse.store(false, Ordering::SeqCst);
        assert_eq!(engine.resume_gpu(), Ok(None), "a later resume asks again and the app takes it");
        assert_eq!(rendered(&engine), json!("gpu"));
    }

    #[test]
    fn an_app_that_panics_taking_the_device_starts_and_answers_on_the_cpu() {
        let (app, shared) = Probe::new();
        shared.panic_on_attach.store(true, Ordering::SeqCst);
        let engine = Engine::start(app, GpuChoice::Preferred).expect("the engine starts");
        assert_eq!(rendered(&engine), json!("cpu"));
        let note = "attach the app could not take the GPU: the app panicked taking the device".to_string();
        assert_eq!(shared.events(), [attached_host(), note, "call render".into()], "the panic is the app's CPU note");
    }

    /// The first failure a notice reports, planted or raised by the device itself, drops the GPU
    /// before the next call: the drop is in the app's log between the last call and the next, the
    /// render is on the CPU from then on, and a second report changes nothing.
    #[test]
    fn the_first_failure_drops_the_gpu_before_the_next_call() {
        let causes: [(&str, fn(&Shared)); 2] = [
            ("a planted report", |shared| {
                assert!(shared.notice(0).report("planted device loss".to_string()));
            }),
            ("a destroyed device", |shared| {
                let device = shared.devices.lock().unwrap().first().cloned().expect("the app kept the device");
                device.destroy();
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
                assert!(shared.notice(0).failed(), "the device-lost callback reached the notice");
            }),
        ];
        for (name, cause) in causes {
            let (app, shared) = Probe::new();
            shared.keep_devices.store(true, Ordering::SeqCst);
            let engine = Engine::start(app, GpuChoice::Preferred).expect("the engine starts");
            assert_eq!(rendered(&engine), json!("gpu"), "{name}: before the failure the render is on the GPU");
            cause(&shared);
            let before = shared.submissions.load(Ordering::SeqCst);
            assert_eq!(rendered(&engine), json!("cpu"), "{name}: the call after the failure renders on the CPU");
            assert_eq!(rendered(&engine), json!("cpu"));
            let mut expected = after_one_render();
            expected.extend(["drop_gpu", "call render", "call render"].map(String::from));
            assert_eq!(shared.events(), expected, "{name}: the GPU is dropped once, between the call before the failure and the one after");
            assert_eq!(shared.submissions.load(Ordering::SeqCst), before, "{name}: no GPU work after the failure");
            shared.notice(0).report("a later report".to_string());
            assert_eq!(rendered(&engine), json!("cpu"));
            assert_eq!(shared.events().iter().filter(|event| *event == "drop_gpu").count(), 1, "{name}: a second report drops nothing again");
        }
    }

    /// One kind of call the engine makes, and what the app logs of it.
    type Call = (&'static str, Box<dyn Fn(&Engine)>, Vec<String>);

    /// The GPU is dropped before the call after a failure whatever that call is: every way into the
    /// app, an export (which asks `info` first) included, and the engine's own GPU calls, so none of
    /// them starts on the failed device.
    #[test]
    fn a_failure_drops_the_gpu_before_every_kind_of_call() {
        let calls: [Call; 6] = [
            ("request", Box::new(|engine| drop(rendered(engine))), vec!["call render".into()]),
            ("request_with_progress", Box::new(|engine| drop(exported(engine))), vec!["call info".into(), "call export with progress".into()]),
            ("frame", Box::new(|engine| drop(engine.frame(json!({}), PixelFormat::Rgba8Straight))), vec!["call frame".into()]),
            ("mcp", Box::new(|engine| drop(engine.mcp("ping"))), vec!["call mcp".into()]),
            ("suspend_gpu", Box::new(|engine| drop(engine.suspend_gpu())), vec!["drop_gpu".into()]),
            ("resume_gpu", Box::new(|engine| drop(engine.resume_gpu())), vec![attached_host()]),
        ];
        for (name, call, logged) in calls {
            let (app, shared) = Probe::new();
            let engine = Engine::start(app, GpuChoice::Preferred).expect("the engine starts");
            assert_eq!(rendered(&engine), json!("gpu"));
            assert!(shared.notice(0).report("planted".to_string()));
            call(&engine);
            let mut expected = after_one_render();
            expected.push("drop_gpu".into());
            expected.extend(logged);
            assert_eq!(shared.events(), expected, "{name}: the GPU is dropped before the call runs");
        }
    }

    /// What a test does to the engine while its export is held mid-way, answering the suspend it
    /// made, if it made one.
    type WhileHeld = (&'static str, fn(&Engine, &Shared) -> Option<Answer<()>>);

    /// A suspend made while an export runs waits for it (plan §16 behaviour 3), since the engine
    /// cannot reach the session inside the export: the export goes on rendering on its device to its
    /// end, by the app's own count, and the suspend runs after it and before the call made after it,
    /// which answers on the CPU. A failure reported mid-export is kept the same way, by the notice,
    /// and the GPU is dropped before the first call after the export: no report is lost between two
    /// calls.
    #[test]
    fn what_arrives_during_an_export_waits_for_it_and_drops_the_gpu_before_the_next_call() {
        let arrivals: [WhileHeld; 2] = [
            ("a suspend", |engine, _| Some(engine.send_suspend_gpu().expect("the suspend is handed over"))),
            ("a failure", |_, shared| {
                assert!(shared.notice(0).report("planted mid-export".to_string()));
                None
            }),
        ];
        for (name, arrive) in arrivals {
            let (app, shared) = Probe::new();
            shared.hold_export.store(true, Ordering::SeqCst);
            let engine = Engine::start(app, GpuChoice::Preferred).expect("the engine starts");
            let export = engine.send_request_with_progress("export", json!({}), |_, _, _| {}, &CancelToken::new()).expect("the export is handed over");
            assert!(shared.held.wait(), "{name}: the export reached its first beat");
            let suspend = arrive(&engine, &shared);
            let render = engine.send_request("render", json!({})).expect("the render is handed over");
            shared.release.open();
            assert!(shared.drained.wait(), "{name}: the export's next beat took the calls in");
            assert!(!shared.events().contains(&"drop_gpu".to_string()), "{name}: nothing drops the GPU inside the export");
            assert_eq!(shared.submissions.load(Ordering::SeqCst), 2, "{name}: the export rendered on the GPU after the arrival");
            shared.finish.open();
            assert_eq!(export.wait(), Ok(json!("gpu")), "{name}");
            if let Some(suspend) = suspend {
                assert_eq!(suspend.wait(), Ok(()), "{name}");
            }
            assert_eq!(render.wait(), Ok(json!("cpu")), "{name}: the call made after it renders on the CPU");
            assert_eq!(shared.submissions.load(Ordering::SeqCst), 2, "{name}: no GPU work after the export");
            let mut expected = vec![attached_host()];
            expected.extend(["call info", "call export with progress", "drop_gpu", "call render"].map(String::from));
            assert_eq!(shared.events(), expected, "{name}: the GPU goes between the export and the next call");
        }
    }

    #[test]
    fn suspending_submits_no_gpu_work_and_every_render_answers_on_the_cpu() {
        let (app, shared) = Probe::new();
        let engine = Engine::start(app, GpuChoice::Preferred).expect("the engine starts");
        assert_eq!(rendered(&engine), json!("gpu"));
        assert_eq!(shared.submissions.load(Ordering::SeqCst), 1, "the counter counts the GPU's work");
        engine.suspend_gpu().expect("the engine suspends");
        for _ in 0..3 {
            assert_eq!(rendered(&engine), json!("cpu"));
        }
        assert_eq!(shared.submissions.load(Ordering::SeqCst), 1, "no GPU work was submitted after the suspend");
        engine.suspend_gpu().expect("suspending twice is not an error");
        assert_eq!(rendered(&engine), json!("cpu"));
    }

    #[test]
    fn resuming_asks_for_a_device_again_and_hands_it_over() {
        let (app, shared) = Probe::new();
        let engine = Engine::start(app, GpuChoice::Preferred).expect("the engine starts");
        engine.suspend_gpu().expect("the engine suspends");
        assert_eq!(engine.resume_gpu(), Ok(None), "a resume that got a device answers no CPU note");
        assert_eq!(rendered(&engine), json!("gpu"), "the app holds a device again");
        assert_eq!(shared.submissions.load(Ordering::SeqCst), 1);
        assert_eq!(shared.events(), [attached_host(), "drop_gpu".into(), attached_host(), "call render".into()]);
    }

    #[test]
    fn a_resume_that_holds_a_device_keeps_it() {
        let (app, shared) = Probe::new();
        let engine = Engine::start(app, GpuChoice::Preferred).expect("the engine starts");
        assert_eq!(engine.resume_gpu(), Ok(None), "a resume with no suspend");
        assert_eq!(engine.resume_gpu(), Ok(None), "and again");
        assert_eq!(shared.events(), [attached_host()], "a working device is never replaced");
    }

    #[test]
    fn a_resume_with_no_device_answers_the_cpu_note() {
        let (app, shared) = Probe::new();
        let engine = Engine::start(app, GpuChoice::CpuOnly).expect("the engine starts");
        engine.suspend_gpu().expect("the engine suspends");
        let reason = "the app asked for CPU-only rendering".to_string();
        assert_eq!(engine.resume_gpu(), Ok(Some(reason.clone())), "no device is no failure, and the caller hears why");
        assert_eq!(rendered(&engine), json!("cpu"));
        let note = format!("attach {reason}");
        assert_eq!(shared.events(), [note.clone(), "drop_gpu".into(), note, "call render".into()], "a CpuOnly engine asks for no device on resume either");
        assert_eq!(shared.submissions.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_resume_after_a_failure_brings_a_fresh_device() {
        let (app, shared) = Probe::new();
        let engine = Engine::start(app, GpuChoice::Preferred).expect("the engine starts");
        assert_eq!(rendered(&engine), json!("gpu"));
        shared.notice(0).report("planted".to_string());
        assert_eq!(rendered(&engine), json!("cpu"));
        assert_eq!(engine.resume_gpu(), Ok(None), "the engine resumes on a device");
        assert_eq!(rendered(&engine), json!("gpu"));
        assert!(!shared.notice(1).failed(), "the new device has a notice of its own");
    }

    #[test]
    fn a_resume_the_app_cannot_take_throws_and_the_next_asks_again() {
        let (app, shared) = Probe::new();
        let engine = Engine::start(app, GpuChoice::Preferred).expect("the engine starts");
        engine.suspend_gpu().expect("the engine suspends");
        shared.refuse.store(true, Ordering::SeqCst);
        assert_eq!(engine.resume_gpu(), Err(CraftError::gpu("the app cannot take this device")));
        assert_eq!(rendered(&engine), json!("cpu"));
        shared.refuse.store(false, Ordering::SeqCst);
        assert_eq!(engine.resume_gpu(), Ok(None), "the next resume asks again");
        assert_eq!(rendered(&engine), json!("gpu"));
    }

    #[test]
    fn the_calls_are_refused_after_close() {
        let (app, _) = Probe::new();
        let engine = Engine::start(app, GpuChoice::Preferred).expect("the engine starts");
        engine.close().expect("the engine closes");
        assert_eq!(engine.suspend_gpu(), Err(CraftError::internal("engine is closed")));
        assert_eq!(engine.resume_gpu(), Err(CraftError::internal("engine is closed")));
    }

    /// A surface that offers what the platform asks and shows nothing, which says when it is dropped
    /// and which of its calls it was handed.
    struct Flagged(Arc<AtomicBool>, Arc<Mutex<Vec<&'static str>>>);

    impl Drop for Flagged {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    impl Swapchain for Flagged {
        fn capabilities(&self, _adapter: &wgpu::Adapter) -> wgpu::SurfaceCapabilities {
            wgpu::SurfaceCapabilities {
                formats: vec![wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm],
                present_modes: vec![wgpu::PresentMode::Fifo],
                alpha_modes: vec![wgpu::CompositeAlphaMode::Opaque],
                ..Default::default()
            }
        }

        fn configure(&self, _device: &wgpu::Device, _configuration: &wgpu::SurfaceConfiguration) {
            self.1.lock().unwrap().push("configure");
        }

        fn acquire(&self) -> wgpu::CurrentSurfaceTexture {
            self.1.lock().unwrap().push("acquire");
            wgpu::CurrentSurfaceTexture::Timeout
        }

        fn current_size(&self) -> (u32, u32) {
            self.1.lock().unwrap().push("current_size");
            (4, 4)
        }
    }

    /// The engine holds the app's surface as a boxed swapchain, so the box must hand each call to the
    /// surface inside it, or the platform's surface is never configured or drawn on.
    #[test]
    fn a_boxed_swapchain_hands_every_call_to_the_surface_inside() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let swapchain: Box<dyn Swapchain + Send> = Box::new(Flagged(Arc::default(), calls.clone()));
        let rig = rig();
        assert_eq!(swapchain.capabilities(&rig.adapter).present_modes, [wgpu::PresentMode::Fifo]);
        let configuration = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Bgra8Unorm,
            width: 4,
            height: 4,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::default(),
        };
        swapchain.configure(&rig.device, &configuration);
        assert!(matches!(swapchain.acquire(), wgpu::CurrentSurfaceTexture::Timeout));
        assert_eq!(swapchain.current_size(), (4, 4));
        assert_eq!(*calls.lock().unwrap(), ["configure", "acquire", "current_size"]);
    }

    /// A platform's call that makes no surface and flips `made` if it is ever run.
    fn noting_maker(made: &Arc<AtomicBool>) -> SurfaceMaker {
        let made = made.clone();
        Box::new(move |_| {
            made.store(true, Ordering::SeqCst);
            Err(CraftError::gpu("made"))
        })
    }

    /// A platform's call that makes a [`Flagged`] surface that flips `dropped` when it goes.
    fn flagged(dropped: Arc<AtomicBool>) -> SurfaceMaker {
        Box::new(move |gpu| Surface::new(Box::new(Flagged(dropped, Arc::default())), gpu))
    }

    /// A lifecycle on a device, holding a preview surface that flips `dropped` when it goes.
    fn with_preview() -> (GpuLifecycle, Probe, Arc<Shared>, Arc<AtomicBool>) {
        let (mut app, shared) = Probe::new();
        let mut lifecycle = GpuLifecycle::start(&mut app, GpuChoice::Preferred);
        let dropped = Arc::new(AtomicBool::new(false));
        lifecycle.attach_preview(flagged(dropped.clone())).expect("the surface attaches");
        (lifecycle, app, shared, dropped)
    }

    /// An engine on a device, previewing into a surface that flips `dropped` when it goes.
    fn engine_with_preview() -> (Engine, Arc<Shared>, Arc<AtomicBool>) {
        let (app, shared) = Probe::new();
        let engine = Engine::start(app, GpuChoice::Preferred).expect("the engine starts");
        let dropped = Arc::new(AtomicBool::new(false));
        engine.attach_preview(flagged(dropped.clone())).expect("the surface attaches on the engine's device");
        (engine, shared, dropped)
    }

    #[test]
    fn suspending_drops_the_preview_surface() {
        let (mut lifecycle, mut app, _, dropped) = with_preview();
        lifecycle.settle(&mut app);
        assert!(!dropped.load(Ordering::SeqCst), "a healthy device keeps its surface");
        assert!(lifecycle.held.is_some(), "the engine holds the device it handed over");
        lifecycle.suspend(&mut app);
        assert!(dropped.load(Ordering::SeqCst), "the surface is dropped with the device");
        assert!(lifecycle.preview.is_none());
        assert!(lifecycle.held.is_none(), "the engine's own handle to the device is dropped, so nothing of it can submit work");
    }

    #[test]
    fn a_failure_drops_the_preview_surface_with_the_gpu() {
        let (mut lifecycle, mut app, shared, dropped) = with_preview();
        shared.notice(0).report("planted".to_string());
        lifecycle.settle(&mut app);
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(shared.events().last().map(String::as_str), Some("drop_gpu"));
    }

    /// The engine's report of its preview surface is settled as every call is: a device whose notice
    /// reported a failure since the last call is dropped, and its surface with it, before the report is
    /// read, so a surface lost with its GPU never reads as held, though no call reached the app since.
    #[test]
    fn a_device_lost_since_the_last_call_takes_the_reported_surface_with_it() {
        let (engine, shared, dropped) = engine_with_preview();
        assert_eq!(engine.has_preview_surface(), Ok(true), "the attached surface is held");
        assert!(shared.notice(0).report("planted device loss".to_string()));
        assert_eq!(engine.has_preview_surface(), Ok(false), "the surface went with the lost device before the report was read");
        assert!(dropped.load(Ordering::SeqCst), "the surface itself was dropped");
        assert_eq!(shared.events(), [attached_host(), "drop_gpu".to_string()], "the report's own settle dropped the GPU, once");
    }

    /// A preview attached, presented, dropped with the GPU on a suspend and attached again after a
    /// resume: the engine's one device carries it, nothing brings it back but the app's own attach,
    /// and while there is no device every attach and present answers so and makes nothing.
    #[test]
    fn a_preview_is_dropped_on_suspend_and_attaches_again_after_resume() {
        let (engine, shared, dropped) = engine_with_preview();
        assert_eq!(engine.present(json!({})), Ok(()));
        engine.suspend_gpu().expect("the GPU is given up");
        assert!(dropped.load(Ordering::SeqCst), "the surface goes with the device");
        let no_surface = Err(CraftError::gpu("no surface"));
        assert_eq!(engine.present(json!({})), no_surface);
        let made = Arc::new(AtomicBool::new(false));
        let refused = engine.attach_preview(noting_maker(&made));
        assert_eq!(refused, Err(CraftError::gpu(NO_DEVICE)), "no device, nothing to preview on");
        assert!(!made.load(Ordering::SeqCst), "no surface is made without a device");
        assert_eq!(engine.resume_gpu(), Ok(None));
        assert_eq!(engine.present(json!({})), no_surface, "a resume does not bring the surface back");
        let again = Arc::new(AtomicBool::new(false));
        engine.attach_preview(flagged(again.clone())).expect("the app attaches its surface again");
        assert_eq!(engine.present(json!({})), Ok(()));
        assert!(!again.load(Ordering::SeqCst));
        // Only the two with a surface ran the app's present.
        let presents = shared.events().iter().filter(|event| *event == "present").count();
        assert_eq!(presents, 2, "a present with no surface never reaches the app");
    }

    /// An engine on the CPU has no device to make a surface on: the attach answers why and the
    /// platform's call to make one never runs.
    #[test]
    fn an_attach_without_a_device_answers_why_and_makes_nothing() {
        let (app, _) = Probe::new();
        let engine = Engine::start(app, GpuChoice::CpuOnly).expect("the engine starts");
        let made = Arc::new(AtomicBool::new(false));
        let refused = engine.attach_preview(noting_maker(&made));
        assert_eq!(refused, Err(CraftError::gpu(NO_DEVICE)));
        assert!(!made.load(Ordering::SeqCst), "no surface is made without a device");
    }

    /// An attach gives up the surface attached before it makes its own, so one that fails leaves no
    /// surface rather than the old one showing, and a detach, twice over, empties the slot.
    #[test]
    fn an_attach_gives_up_the_surface_attached_before_it_makes_its_own() {
        let (mut app, _) = Probe::new();
        let mut lifecycle = GpuLifecycle::start(&mut app, GpuChoice::Preferred);
        let first = Scripted::new((8, 8), []);
        let watch = first.clone();
        lifecycle.attach_preview(Box::new(move |gpu| Surface::new(Box::new(first), gpu))).expect("the first surface attaches");
        assert_eq!(watch.holders(), 2, "the lifecycle holds the first surface");
        let held_when_made = Arc::new(Mutex::new(None));
        let noted = held_when_made.clone();
        let refused = lifecycle.attach_preview(Box::new(move |_| {
            *noted.lock().unwrap() = Some(watch.holders());
            Err(CraftError::gpu("the platform's handle cannot become a surface"))
        }));
        assert_eq!(*held_when_made.lock().unwrap(), Some(1), "the first surface was given up before the second was made");
        assert_eq!(refused, Err(CraftError::gpu("the platform's handle cannot become a surface")));
        assert!(lifecycle.preview.is_none(), "a failed attach leaves no surface, the old one included");
        lifecycle.attach_preview(flagged(Arc::default())).expect("a new surface attaches");
        lifecycle.detach_preview();
        lifecycle.detach_preview();
        assert!(lifecycle.preview.is_none(), "a detach drops the surface and a second changes nothing");
    }
}
