//! `mmxrs-launcher`: the window. `--print-args` prints the command it would
//! run from the saved config and exits (docs/launcher.md).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::Child;
use std::time::{Duration, Instant};

use mmx_launcher::*;

fn main() -> eframe::Result {
    if std::env::args().any(|a| a == "--print-args") {
        let s = Settings::load(&Settings::path());
        match build_args(&s) {
            Ok(a) => println!("{}", format_command(&game_binary().to_string_lossy(), &a)),
            Err(e) => {
                eprintln!("cannot launch: {e} (config {})", Settings::path().display());
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("mmxrs launcher")
            .with_app_id("mmxrs-launcher")
            .with_inner_size([520.0, 640.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "mmxrs launcher",
        opts,
        Box::new(|cc| {
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            Ok(Box::new(App {
                s: Settings::load(&Settings::path()),
                roms: Default::default(),
                child: None,
                status: String::new(),
                log: Vec::new(),
                log_at: Instant::now(),
            }))
        }),
    )
}

/// Cached header check of one ROM path.
#[derive(Default)]
struct RomCheck {
    path: String,
    result: Option<Result<RomInfo, String>>,
}

impl RomCheck {
    fn get(&mut self, path: &str, x3: bool) -> Option<&Result<RomInfo, String>> {
        if path.trim().is_empty() {
            return None;
        }
        if self.result.is_none() || self.path != path {
            self.path = path.to_string();
            self.result = Some(check_rom(path, x3));
        }
        self.result.as_ref()
    }
}

struct App {
    s: Settings,
    /// [X, X3].
    roms: [RomCheck; 2],
    child: Option<Child>,
    status: String,
    log: Vec<String>,
    log_at: Instant,
}

fn log_path() -> PathBuf {
    data_dir().join(LOG_FILE_NAME)
}

fn rom_row(ui: &mut egui::Ui, label: &str, path: &mut String, check: &mut RomCheck, x3: bool) -> bool {
    ui.strong(label);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(path).desired_width(ui.available_width() - 90.0));
        if ui.button("Browse…").clicked() {
            if let Some(p) = rfd::FileDialog::new()
                .add_filter("SNES ROM", &["sfc", "smc", "SFC", "SMC"])
                .add_filter("All files", &["*"])
                .pick_file()
            {
                *path = p.to_string_lossy().into_owned();
            }
        }
    });
    match check.get(path, x3) {
        None => {
            ui.weak("not set (browse or drop the file on the window)");
            false
        }
        Some(Ok(i)) => {
            ui.label(format!("{}  v1.{}  sha256 {}…", i.title, i.version, &i.sha256[..16]))
                .on_hover_text(&i.sha256);
            true
        }
        Some(Err(e)) => {
            ui.colored_label(ui.visuals().error_fg_color, e.as_str());
            false
        }
    }
}

impl App {
    fn launch(&mut self) {
        let args = match build_args(&self.s) {
            Ok(a) => a,
            Err(e) => return self.status = e,
        };
        let _ = self.s.save(&Settings::path());
        let bin = game_binary();
        match spawn(&bin, &args, &log_path()) {
            Ok(c) => {
                self.status = format!("running: {}", format_command(&bin.to_string_lossy(), &args));
                self.child = Some(c);
            }
            Err(e) => self.status = format!("could not start {}: {e}", bin.display()),
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(c) = &mut self.child {
            match c.try_wait() {
                Ok(None) => ctx.request_repaint_after(Duration::from_millis(500)),
                Ok(Some(st)) => {
                    self.status = format!("game exited ({st})");
                    self.child = None;
                }
                Err(e) => {
                    self.status = format!("lost the game process: {e}");
                    self.child = None;
                }
            }
            if self.log_at.elapsed() > Duration::from_millis(400) || self.child.is_none() {
                self.log_at = Instant::now();
                let text = std::fs::read_to_string(log_path()).unwrap_or_default();
                self.log = last_lines(&text, 8).into_iter().map(String::from).collect();
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        // Dropped files: an X3 title goes to the X3 slot, anything else to X.
        for f in ctx.input(|i| i.raw.dropped_files.clone()) {
            let p = f.path().to_string_lossy().into_owned();
            if !p.is_empty() {
                let is_x3 = std::fs::read(&p).ok().and_then(|b| rom_info(&b).ok()).is_some_and(|i| i.title == "MEGAMAN X3");
                *(if is_x3 { &mut self.s.x3_rom } else { &mut self.s.rom }) = p;
            }
        }
        egui::CentralPanel::default_margins().show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.body(ui));
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.s.save(&Settings::path());
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }
}

impl App {
    fn body(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.s;
        ui.heading("ROMs");
        let [rx, rx3] = &mut self.roms;
        let x_ok = rom_row(ui, "Mega Man X (USA)", &mut s.rom, rx, false);
        let x3_ok = rom_row(ui, "Mega Man X3 (USA), optional: Zero's sprites + saber", &mut s.x3_rom, rx3, true);
        ui.separator();

        ui.heading("Options");
        ui.add(egui::Slider::new(&mut s.scale, 1..=8).text("window scale"));
        ui.checkbox(&mut s.fullscreen, "Fullscreen");
        ui.checkbox(&mut s.zero, "Play as Zero (needs X3)");
        ui.add_enabled(!s.online, egui::Checkbox::new(&mut s.coop_local, "Local co-op (2 players)"));
        ui.separator();

        egui::CollapsingHeader::new("Play online").default_open(s.online).show(ui, |ui| {
            ui.checkbox(&mut s.online, "Online co-op");
            ui.add_enabled_ui(s.online, |ui| {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut s.host, true, "Host");
                    ui.radio_value(&mut s.host, false, "Join");
                });
                ui.horizontal(|ui| {
                    ui.label("Room");
                    ui.text_edit_singleline(&mut s.room);
                    if ui.button("random").clicked() {
                        s.room = random_room();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Signal");
                    ui.text_edit_singleline(&mut s.signal);
                });
                ui.horizontal(|ui| {
                    ui.label("Mode");
                    for m in [Mode::Rollback, Mode::Lockstep] {
                        ui.radio_value(&mut s.mode, m, m.name());
                    }
                    if s.delay.is_some_and(|d| d > s.mode.max_delay()) {
                        s.delay = None;
                    }
                    ui.label("Delay");
                    let shown = s.delay.map_or("auto".into(), |d| d.to_string());
                    egui::ComboBox::from_id_salt("delay").selected_text(shown).show_ui(ui, |ui| {
                        ui.selectable_value(&mut s.delay, None, "auto");
                        for d in 0..=s.mode.max_delay() {
                            ui.selectable_value(&mut s.delay, Some(d), d.to_string());
                        }
                    });
                });
                if s.mode == Mode::Rollback {
                    ui.horizontal(|ui| {
                        ui.label("Prediction");
                        let shown = s.prediction.map_or("auto (8)".into(), |p| p.to_string());
                        egui::ComboBox::from_id_salt("prediction").selected_text(shown).show_ui(ui, |ui| {
                            ui.selectable_value(&mut s.prediction, None, "auto (8)");
                            for p in [4u8, 8, 12, 16, 20, 25, 30] {
                                ui.selectable_value(&mut s.prediction, Some(p), p.to_string());
                            }
                        });
                        ui.weak("frames of round trip to absorb; ~20 across continents");
                    });
                } else {
                    s.prediction = None;
                }
                ui.horizontal(|ui| {
                    ui.label("ICE");
                    ui.add(egui::TextEdit::singleline(&mut s.ice).hint_text("game default; or none, or stun:.. turn:.. username=U credential=C"));
                });
            });
            ui.weak("Host and guest need identical ROM dumps and the same mode.");
        });
        ui.separator();

        let args = build_args(s);
        let zero_ok = !s.zero || x3_ok;
        let can = x_ok && zero_ok && args.is_ok() && self.child.is_none();
        ui.horizontal(|ui| {
            if ui.add_enabled(can, egui::Button::new("Launch")).clicked() {
                self.launch();
            }
            if let Some(c) = &mut self.child {
                if ui.button("Stop").clicked() {
                    let _ = c.kill();
                }
            }
        });
        if let Err(e) = &args {
            ui.weak(e.as_str());
        } else if !zero_ok {
            ui.weak("playing as Zero needs a valid X3 ROM");
        }
        if !self.status.is_empty() {
            ui.label(&self.status);
        }
        if !self.log.is_empty() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                for l in &self.log {
                    ui.monospace(l);
                }
            });
        }
    }
}
