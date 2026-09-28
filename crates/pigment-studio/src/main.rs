//! `pigment-studio`: the Pigment Prose desktop app (tasks 11–12).
//!
//! The painting adapter is chosen by `pigment_gpu::adapter::select` (the
//! capability service: portable limits, software adapters refused unless
//! allowed). The window's device is created by egui on an adapter that can
//! present to it, preferring the painting adapter; when they are the same
//! adapter the painter shares that device, and otherwise it opens its own
//! (multi-GPU systems, docs/studio.md). If no acceptable adapter exists the app still
//! opens a window, on whatever wgpu can create, to explain what is wrong;
//! it never presents a software rasterizer as GPU acceleration.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
use pigment_core::capability::{AdapterPolicy, choose_display, same_adapter};
use pigment_gpu::{GpuContext, PaintRenderer, adapter};

use pigment_studio::app::{StudioApp, StudioOptions};
use pigment_studio::export::Exporter;
use pigment_studio::files::NativeDialogs;
use pigment_studio::script::{self, Report};
use pigment_studio::worker::PreviewWorker;

const USAGE: &str = "\
pigment-studio [options]

  --adapter NAME         paint on the adapter whose name contains NAME
  --display-adapter NAME show the window on the adapter whose name contains
                         NAME (default: the best adapter that can present;
                         when it is the painting adapter, one device is shared)
  --separate-display     give the painter its own device even when the window
                         shows on the same adapter (for measurements)
  --allow-software       accept a software rasterizer (labelled as such)
  --gl                   also consider OpenGL adapters
  --preview-delay-ms MS  simulate a slow GPU: every preview render waits MS
  --lose-device-after N  simulate a GPU reset after N previews
  --log-timings          print one line per displayed preview
  --script               run the scripted responsiveness check, then close;
                         exits non-zero if a check fails
  --screenshot FILE.png  where --script saves its capture of the window
";

#[derive(Debug, Default)]
struct Args {
    policy: AdapterPolicy,
    opts: StudioOptions,
    /// Measurement aid: give the painter its own device even when the window
    /// shows on the same adapter.
    separate_display: bool,
    /// Show the window on the adapter whose name contains this.
    display_filter: Option<String>,
}

fn parse() -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--adapter" => a.policy.name_filter = Some(value("--adapter")?),
            "--allow-software" => a.policy.allow_software = true,
            "--gl" => a.policy.include_gl = true,
            "--preview-delay-ms" => {
                let v = value("--preview-delay-ms")?;
                let ms: u64 = v
                    .parse()
                    .map_err(|_| format!("bad --preview-delay-ms {v:?}"))?;
                a.opts.worker.delay = Duration::from_millis(ms);
            }
            "--lose-device-after" => {
                let v = value("--lose-device-after")?;
                a.opts.worker.lose_device_after = Some(
                    v.parse()
                        .map_err(|_| format!("bad --lose-device-after {v:?}"))?,
                );
            }
            "--log-timings" => a.opts.log_timings = true,
            "--separate-display" => a.separate_display = true,
            "--display-adapter" => a.display_filter = Some(value("--display-adapter")?),
            "--script" => a.opts.script = true,
            "--screenshot" => a.opts.screenshot = Some(PathBuf::from(value("--screenshot")?)),
            "--help" | "-h" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument {other:?}\n\n{USAGE}")),
        }
    }
    Ok(a)
}

fn native_options(wgpu_options: WgpuConfiguration) -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Pigment Prose")
            .with_app_id("pigment-prose")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([760.0, 480.0]),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options,
        persist_window: false,
        ..Default::default()
    }
}

/// The window and the painter share one device, so the chosen adapter must
/// also be able to present to the display. On multi-GPU systems an adapter
/// with no display attached (for example an iGPU when the monitors are on
/// the discrete card) cannot, and egui-wgpu panics while creating the
/// surface. Say what happened instead of showing only the panic.
/// The adapter chosen to show the window, for the panic explanation.
static DISPLAY_ADAPTER: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn explain_surface_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        if msg.contains("surface isn't supported") {
            let on = DISPLAY_ADAPTER
                .get()
                .map_or("the chosen GPU", String::as_str);
            eprintln!(
                "pigment-studio: the window could not be shown on {on}: its driver accepted \
                 the window but cannot present to the display in use. Choose the GPU that \
                 drives your monitor with --display-adapter NAME (see `pigment-prose gpu-info`)."
            );
            std::process::exit(2);
        }
        default(info);
    }));
}

fn main() -> ExitCode {
    explain_surface_panics();
    let args = match parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    // Choose the painting adapter now, so a missing or software-only GPU is
    // explained in a window before anything else.
    let painter = match adapter::select(&adapter::instance(args.policy.include_gl), &args.policy) {
        Ok((_, r)) => r,
        Err(e) => {
            eprintln!("pigment-studio: {e}");
            return init_failure(&args, e.to_string());
        }
    };
    let report = Arc::new(Mutex::new(Report {
        delayed: !args.opts.worker.delay.is_zero(),
        device_loss_expected: args.opts.worker.lose_device_after.is_some(),
        ..Default::default()
    }));
    // The window shows on an adapter that can present to it: the painter's
    // own if it can (then both share one device), otherwise the best one
    // that can. On a multi-GPU system the painting may run on one GPU and
    // the window on another; previews cross by CPU readback either way.
    let selector_painter = painter.clone();
    let allow_software = args.policy.allow_software;
    let display_filter = args.display_filter.clone().map(|f| f.to_lowercase());
    let setup = WgpuSetup::CreateNew(WgpuSetupCreateNew {
        instance_descriptor: adapter::instance_descriptor(args.policy.include_gl),
        native_adapter_selector: Some(Arc::new(move |adapters, surface| {
            let candidates: Vec<_> = adapters
                .iter()
                .map(|a| {
                    let r = adapter::report(a);
                    let wanted = display_filter
                        .as_ref()
                        .is_none_or(|f| r.name.to_lowercase().contains(f));
                    let presents = wanted && surface.is_none_or(|s| a.is_surface_supported(s));
                    (r, presents)
                })
                .collect();
            let chosen = choose_display(&candidates, &selector_painter, allow_software)
                .map(|i| adapters[i].clone());
            if let Some(a) = &chosen {
                let _ = DISPLAY_ADAPTER.set(a.get_info().name);
            }
            chosen.ok_or_else(|| match &display_filter {
                Some(f) => format!("no adapter named like {f:?} can display the window"),
                None => "no GPU on this system can display the window; check the display driver"
                    .to_string(),
            })
        })),
        device_descriptor: Arc::new(|_| pigment_gpu::context::device_descriptor()),
        ..WgpuSetupCreateNew::without_display_handle()
    });
    let mut opts = args.opts.clone();
    let rep = report.clone();
    let policy = args.policy.clone();
    let separate = args.separate_display;
    let result = eframe::run_native(
        "Pigment Prose",
        native_options(WgpuConfiguration {
            wgpu_setup: setup,
            ..Default::default()
        }),
        Box::new(move |cc| {
            pigment_studio::theme::install(&cc.egui_ctx);
            let rs = cc
                .wgpu_render_state
                .as_ref()
                .ok_or("the window has no wgpu renderer")?;
            let display = adapter::report(&rs.adapter);
            let shared = !separate && same_adapter(&display, &painter);
            // Share the window's device when it is the painting adapter;
            // otherwise open the painter's own device on its adapter.
            let ctx = Arc::new(if shared {
                GpuContext::from_existing(
                    rs.instance.clone(),
                    rs.adapter.clone(),
                    rs.device.clone(),
                    rs.queue.clone(),
                )
            } else {
                GpuContext::new(&policy)?
            });
            // Previews and exports each get a renderer, so an export never
            // queues behind previews or blocks them.
            let renderer = PaintRenderer::new(ctx.clone())?;
            let export_renderer = PaintRenderer::new(ctx.clone())?;
            let caps = ctx.capabilities.clone();
            let display_label = format!("{} ({:?})", display.name, display.backend);
            eprintln!(
                "pigment-studio: painting on {}; window on {display_label} ({})",
                caps.label(),
                if shared {
                    "one shared device"
                } else {
                    "separate devices"
                }
            );
            opts.display = Some((display_label.clone(), shared));
            {
                let mut r = rep.lock().unwrap_or_else(|e| e.into_inner());
                r.device = caps.label();
                r.display = format!(
                    "{display_label} ({})",
                    if shared {
                        "shared device"
                    } else {
                        "separate device"
                    }
                );
            }
            let egui_ctx = cc.egui_ctx.clone();
            let worker =
                PreviewWorker::spawn(renderer, opts.worker, move || egui_ctx.request_repaint());
            let egui_ctx = cc.egui_ctx.clone();
            let dialogs = NativeDialogs::new(move || egui_ctx.request_repaint());
            let egui_ctx = cc.egui_ctx.clone();
            let exporter = Exporter::spawn(export_renderer, move || egui_ctx.request_repaint());
            Ok(Box::new(StudioApp::new(
                worker,
                caps,
                opts,
                rep,
                Box::new(dialogs),
                exporter,
            )))
        }),
    );
    if let Err(e) = result {
        eprintln!("pigment-studio: the window could not be opened: {e}");
        return ExitCode::FAILURE;
    }
    if args.opts.script {
        // The capture is encoded on a background thread; give it a moment.
        for _ in 0..300 {
            let r = report.lock().unwrap_or_else(|e| e.into_inner());
            if r.screenshot.is_some() || args.opts.screenshot.is_none() {
                break;
            }
            drop(r);
            std::thread::sleep(Duration::from_millis(10));
        }
        let (pass, text) = report.lock().unwrap_or_else(|e| e.into_inner()).verdict();
        println!("{text}RESULT: {}", if pass { "PASS" } else { "FAIL" });
        if !pass {
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

/// No acceptable GPU: explain it in a window (drawn by any adapter wgpu can
/// create, possibly a software one) and exit non-zero.
fn init_failure(args: &Args, message: String) -> ExitCode {
    let instance = adapter::instance(true);
    let everything = AdapterPolicy {
        name_filter: None,
        allow_software: true,
        include_gl: true,
    };
    let adapters: Vec<String> = adapter::enumerate(&instance, &everything)
        .into_iter()
        .map(|(_, r)| {
            format!(
                "{} ({:?}, {:?}{})",
                r.name,
                r.backend,
                r.kind,
                if r.software { ", software" } else { "" }
            )
        })
        .collect();
    let screenshot = args
        .opts
        .script
        .then(|| args.opts.screenshot.clone())
        .flatten();
    let app = InitFailure {
        message,
        adapters,
        screenshot,
        frames: 0,
    };
    let shown = eframe::run_native(
        "Pigment Prose",
        native_options(WgpuConfiguration::default()),
        Box::new(|cc| {
            pigment_studio::theme::install(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = shown {
        eprintln!("pigment-studio: could not open a window to show this error either: {e}");
    }
    ExitCode::FAILURE
}

#[derive(Debug)]
struct InitFailure {
    message: String,
    adapters: Vec<String>,
    screenshot: Option<PathBuf>,
    frames: u32,
}

impl eframe::App for InitFailure {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(24.0);
            ui.heading("Pigment Prose needs a hardware GPU");
            ui.add_space(8.0);
            ui.label("The painting renderer could not start:");
            ui.add_space(4.0);
            ui.label(egui::RichText::new(&self.message).monospace());
            ui.add_space(12.0);
            ui.label("Adapters found on this system:");
            if self.adapters.is_empty() {
                ui.label(egui::RichText::new("none").monospace());
            }
            for a in &self.adapters {
                ui.label(egui::RichText::new(format!("• {a}")).monospace());
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Copy details").clicked() {
                    ctx.copy_text(format!("{}\n\n{}", self.message, self.adapters.join("\n")));
                }
                if ui.button("Quit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
        // Scripted capture of this screen for the evidence log.
        if let Some(path) = &self.screenshot {
            self.frames += 1;
            ctx.request_repaint();
            if self.frames == 10 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            let shot = ctx.input(|i| {
                i.raw.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(img) = shot {
                if let Err(e) = script::save_png(path, &img) {
                    eprintln!("screenshot: {e}");
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}
