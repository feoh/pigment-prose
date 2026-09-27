//! `pigment-studio`: the Pigment Prose desktop app (task 11).
//!
//! The GPU is opened once through `pigment_gpu::GpuContext` (the capability
//! service: portable limits, software adapters refused unless allowed) and
//! shared with egui through `WgpuSetup::Existing`, so the window and the
//! painter use one device. If no acceptable adapter exists the app still
//! opens a window, on whatever wgpu can create, to explain what is wrong;
//! it never presents a software rasterizer as GPU acceleration.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupExisting};
use pigment_core::capability::AdapterPolicy;
use pigment_gpu::{GpuContext, PaintRenderer, adapter};

use pigment_studio::app::{StudioApp, StudioOptions};
use pigment_studio::script::{self, Report};
use pigment_studio::worker::PreviewWorker;

const USAGE: &str = "\
pigment-studio [options]

  --adapter NAME         use the adapter whose name contains NAME
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
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 420.0]),
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
            eprintln!(
                "pigment-studio: the selected GPU cannot display this window (its outputs are \
                 not connected to the display in use). Choose the GPU that drives your monitor \
                 with --adapter, or omit --adapter to use the default selection."
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
    let ctx = GpuContext::new(&args.policy).and_then(|ctx| {
        let ctx = Arc::new(ctx);
        let renderer = PaintRenderer::new(ctx.clone())?;
        Ok((ctx, renderer))
    });
    let (ctx, renderer) = match ctx {
        Ok(v) => v,
        Err(e) => {
            eprintln!("pigment-studio: {e}");
            return init_failure(&args, e.to_string());
        }
    };
    let caps = ctx.capabilities.clone();
    eprintln!("pigment-studio: painting on {}", caps.label());
    let report = Arc::new(Mutex::new(Report {
        device: caps.label(),
        delayed: !args.opts.worker.delay.is_zero(),
        device_loss_expected: args.opts.worker.lose_device_after.is_some(),
        ..Default::default()
    }));
    let setup = WgpuSetup::Existing(WgpuSetupExisting {
        instance: ctx.instance.clone(),
        adapter: ctx.adapter.clone(),
        device: ctx.device.clone(),
        queue: ctx.queue.clone(),
    });
    let opts = args.opts.clone();
    let rep = report.clone();
    let result = eframe::run_native(
        "Pigment Prose",
        native_options(WgpuConfiguration {
            wgpu_setup: setup,
            ..Default::default()
        }),
        Box::new(move |cc| {
            let egui_ctx = cc.egui_ctx.clone();
            let worker =
                PreviewWorker::spawn(renderer, opts.worker, move || egui_ctx.request_repaint());
            Ok(Box::new(StudioApp::new(worker, caps, opts, rep)))
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
        Box::new(|_| Ok(Box::new(app))),
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
