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
                failure: None,
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
    /// Last run's failure: plain explanation (if one matched) and exit status.
    failure: Option<Failure>,
    log: Vec<String>,
    log_at: Instant,
}

struct Failure {
    explanation: Option<String>,
    exit: String,
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
            ui.label(format!(
                            "{}  v1.{}  {}  sha256 {}…",
                            i.title,
                            i.version,
                            if i.region >= 2 { "PAL" } else { "NTSC" },
                            &i.sha256[..16]
                        ))
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
        self.failure = None;
        let args = match build_args(&self.s).and_then(|a| preflight(&self.s).map(|()| a)) {
            Ok(a) => a,
            Err(e) => return self.status = e,
        };
        let _ = self.s.save(&Settings::path());
        let bin = game_binary();
        match spawn(&bin, &args, &log_path()) {
            Ok(c) => {
                self.status = format!("running: {}", format_command(&bin.to_string_lossy(), &args));
                self.log.clear();
                self.child = Some(c);
            }
            Err(e) => {
                self.status.clear();
                self.failure = Some(Failure {
                    explanation: Some(format!(
                        "Could not start the game program {} ({e}). Keep mmxrs{} in the same folder as the launcher.",
                        bin.display(),
                        std::env::consts::EXE_SUFFIX
                    )),
                    exit: String::new(),
                });
            }
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(c) = &mut self.child {
            match c.try_wait() {
                Ok(None) => ctx.request_repaint_after(Duration::from_millis(500)),
                Ok(Some(st)) => {
                    self.child = None;
                    if st.success() {
                        self.status = "game closed".into();
                    } else {
                        let text = std::fs::read_to_string(log_path()).unwrap_or_default();
                        self.status.clear();
                        self.failure = Some(Failure {
                            explanation: diagnose(&text, &self.s),
                            exit: match st.code() {
                                Some(code) => format!("The game stopped with exit code {code}."),
                                None => format!("The game stopped ({st})."),
                            },
                        });
                    }
                }
                Err(e) => {
                    self.status = format!("lost the game process: {e}");
                    self.child = None;
                }
            }
            if self.log_at.elapsed() > Duration::from_millis(400) || self.child.is_none() {
                self.log_at = Instant::now();
                let text = std::fs::read_to_string(log_path()).unwrap_or_default();
                let n = if self.failure.is_some() { 14 } else { 8 };
                self.log = last_lines(&text, n).into_iter().map(String::from).collect();
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
        let x3_ok = rom_row(ui, "Mega Man X3 (USA), optional: alternate Zero art + saber", &mut s.x3_rom, rx3, true);
        ui.separator();

        ui.heading("Options");
        ui.add(egui::Slider::new(&mut s.scale, 1..=8).text("window scale"));
        ui.checkbox(&mut s.fullscreen, "Fullscreen");
        ui.horizontal(|ui| {
            ui.label("Widescreen");
            let shown = if s.widescreen.is_empty() { "off" } else { s.widescreen.as_str() }.to_string();
            egui::ComboBox::from_id_salt("widescreen").selected_text(shown).show_ui(ui, |ui| {
                for w in ["off", "16:10", "16:9", "21:9", "auto"] {
                    ui.selectable_value(&mut s.widescreen, w.to_string(), w);
                }
            });
        })
        .response
        .on_hover_text("extra scenery left and right of the picture; F10 in game cycles it");
        ui.checkbox(&mut s.wide_gameplay, "spawn enemies at the wide edges (changes gameplay; both players)")
            .on_hover_text("enemies spawn, act and leave 136-160 px further out on both sides, whatever the widescreen setting; online, both players must tick it");
        ui.checkbox(&mut s.dynamic_lighting, "Multiply style dynamic lighting");
        ui.checkbox(&mut s.charge_effects, "Buster charge lighting and particles");
        ui.collapsing("Controller buttons", |ui| {
            ui.weak("Pad buttons are positional (South is the bottom face button). \"default\" keeps the game's layout, \"none\" unbinds.");
            ui.weak("The mapping applies to every player's controller, P1 to P4. Each controller joins on its first press as the next player the keyboard isn't playing.");
            egui::Grid::new("pad-map").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                for pad in mmx_launcher::PAD_BUTTONS {
                    ui.label(pad);
                    let current = s.pad_map.get(pad).cloned().unwrap_or_default();
                    let mut chosen = current.clone();
                    egui::ComboBox::from_id_salt(("pad", pad))
                        .selected_text(if chosen.is_empty() { "default" } else { chosen.as_str() })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut chosen, String::new(), "default");
                            for b in mmx_launcher::GAME_BUTTONS {
                                ui.selectable_value(&mut chosen, b.to_string(), b);
                            }
                            ui.selectable_value(&mut chosen, "none".to_string(), "none");
                        });
                    if chosen != current {
                        if chosen.is_empty() {
                            s.pad_map.remove(pad);
                        } else {
                            s.pad_map.insert(pad.to_string(), chosen);
                        }
                    }
                    ui.end_row();
                }
            });
            ui.horizontal(|ui| {
                for (name, map) in mmx_launcher::pad_map_presets() {
                    if ui.button(name).clicked() {
                        s.pad_map = map;
                    }
                }
            });
        });
        ui.checkbox(&mut s.bone_sheets, "60 FPS generated character sprites")
            .on_hover_text("Needs a separate sprite pack folder, not included in the download. Without one the game uses the ROM sprites.");
        if s.bone_sheets {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut s.bone_sheets_path).desired_width(ui.available_width() - 92.0));
                if ui.button("Browse…").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        s.bone_sheets_path = path.to_string_lossy().into_owned();
                    }
                }
            });
            match bone_pack(&s.bone_sheets_path) {
                Ok(found) => {
                    let names: Vec<_> = found.iter().map(|c| c.label()).collect();
                    ui.weak(format!("Pack found: {} characters ({}).", found.len(), names.join(", ")))
                        .on_hover_text("Other characters keep their ROM art; missing clips use the pack's idle pose.");
                }
                Err(e) => {
                    ui.colored_label(ui.visuals().warn_fg_color, e);
                }
            }
        }
        ui.separator();

        // Online remains a two-player mode; local play can use up to four.
        ui.horizontal(|ui| {
            ui.label("Players");
            ui.radio_value(&mut s.online, false, "Local");
            ui.radio_value(&mut s.online, true, "Online co-op");
        });
        if !s.online {
            ui.add(egui::Slider::new(&mut s.local_players, 1..=4).text("local players"));
            s.coop_local = s.local_players > 1;
            let vs_missing = Character::ALL.iter().find_map(|&c| character_unavailable(s, c));
            if vs_missing.is_some() { s.vs_mode = false; }
            ui.add_enabled(vs_missing.is_none(), egui::Checkbox::new(&mut s.vs_mode, "VS arena match"))
                .on_hover_text("Choose 2-4 fighters and an arena in a Street Fighter style selection screen.")
                .on_disabled_hover_text(format!(
                    "VS mode offers every fighter, including Bass and Proto Man. {}",
                    vs_missing.unwrap_or_default()
                ));
            if s.vs_mode && s.local_players < 2 {
                s.local_players = 2;
                s.coop_local = true;
            }
            if s.vs_mode { s.bone_sheets = true; }
        }
        let player_count = if s.online { 2 } else { s.local_players };
        for i in 0..usize::from(player_count) {
            ui.horizontal(|ui| {
                ui.label(format!("Player {}", i + 1));
                let selected = Character::parse(&s.characters[i])
                    .map(|character| character.label().to_owned())
                    .unwrap_or_else(|| s.characters[i].clone());
                let blocked: Vec<_> = Character::ALL.iter().map(|&c| character_unavailable(s, c)).collect();
                let response = egui::ComboBox::from_id_salt(("character", i)).selected_text(selected).show_ui(ui, |ui| {
                    for (&character, why) in Character::ALL.iter().zip(&blocked) {
                        let label = if why.is_some() { format!("{} (needs sprite pack)", character.label()) } else { character.label().to_string() };
                        let r = ui.add_enabled(why.is_none(), egui::Button::selectable(s.characters[i] == character.id(), label));
                        if let Some(why) = why {
                            r.on_disabled_hover_text(why);
                        } else if r.clicked() {
                            s.characters[i] = character.id().to_string();
                        }
                    }
                });
                if let Some(why) = Character::parse(&s.characters[i]).and_then(|c| character_unavailable(s, c)) {
                    ui.colored_label(ui.visuals().warn_fg_color, "⚠").on_hover_text(why);
                }
                if i == 0 && response.response.changed() { s.zero = false; }
            });
        }
        if let Some(warning) = coop_art_warning(s) {
            ui.colored_label(ui.visuals().warn_fg_color, format!("⚠ {warning}"));
        }
        if s.characters.iter().take(usize::from(player_count)).any(|c| c == "vile") {
            ui.label("Vile on-foot sprite (optional)");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut s.vile_sprite_path).desired_width(ui.available_width() - 92.0));
                if ui.button("Browse…").clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("PNG", &["png"]).pick_file() {
                        s.vile_sprite_path = path.to_string_lossy().into_owned();
                    }
                }
            });
            if std::path::Path::new(s.vile_sprite_path.trim()).is_file() {
                ui.weak("Used for Vile even when 60 FPS bone animation sheets are off.");
            } else {
                ui.weak("Optional, not included in the download. Without it Vile uses his ROM poses.");
            }
        }
        if s.characters.iter().take(usize::from(player_count)).any(|c| c == "zero") {
            ui.weak("Zero uses his X1 sprites by default. Add X3 for alternate art and saber attacks.");
            ui.horizontal(|ui| {
                ui.label("Custom Zero sprite pack (optional, body art only)");
                ui.text_edit_singleline(&mut s.sprite_pack);
                if ui.button("Browse…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("Sprite pack", &["json"]).pick_file() {
                        s.sprite_pack = p.to_string_lossy().into_owned();
                    }
                }
                if ui.button("Clear").clicked() {
                    s.sprite_pack.clear();
                }
            });
        }
        if s.characters.iter().take(usize::from(player_count)).any(|c| Character::parse(c).is_some_and(|character| !matches!(character, Character::X | Character::Zero) && !needs_bone_sheet(character))) {
            ui.weak("Bosses use their X1 ROM sprites. Missing movement poses use their idle animation.");
        }
        if s.online {
            ui.group(|ui| {
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
                ui.weak("Host and guest need identical ROM dumps and the same mode.");
            });
        }
        ui.separator();

        let args = build_args(s).and_then(|a| preflight(s).map(|()| a));
        let can = x_ok && (s.x3_rom.trim().is_empty() || x3_ok) && args.is_ok() && self.child.is_none();
        let x3_invalid = !s.x3_rom.trim().is_empty() && !x3_ok;
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
            ui.colored_label(ui.visuals().warn_fg_color, e.as_str());
        } else if x3_invalid {
            ui.colored_label(ui.visuals().warn_fg_color, "the selected X3 ROM is invalid: clear it or pick Mega Man X3 (USA)");
        }
        if !self.status.is_empty() {
            ui.label(&self.status);
        }
        if let Some(f) = &self.failure {
            let error = ui.visuals().error_fg_color;
            egui::Frame::group(ui.style()).stroke(egui::Stroke::new(1.0, error)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.colored_label(error, egui::RichText::new("The game could not run").strong());
                if let Some(e) = &f.explanation {
                    ui.label(e);
                }
                if !f.exit.is_empty() {
                    ui.weak(&f.exit);
                }
                if !self.log.is_empty() {
                    ui.weak(format!("Game output ({}):", log_path().display()));
                }
            });
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
