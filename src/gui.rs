//! A small window for people who don't want the command line:
//! drop files in, pick a format, press Convert.

use crate::convert::{self, Format, Options, Report, FORMATS};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;

pub fn run() -> ExitCode {
    hide_console();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("avi2mp4")
            .with_inner_size([560.0, 420.0])
            .with_min_inner_size([400.0, 300.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    match eframe::run_native("avi2mp4", options, Box::new(|_| Ok(Box::new(App::new())))) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: could not open the window: {e}");
            ExitCode::FAILURE
        }
    }
}

/// When the exe was double-clicked, Windows gives it its own console window.
/// Close that one; leave it alone when started from a terminal.
fn hide_console() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
            fn FreeConsole() -> i32;
        }
        let mut ids = [0u32; 2];
        unsafe {
            if GetConsoleProcessList(ids.as_mut_ptr(), 2) == 1 {
                FreeConsole();
            }
        }
    }
}

enum State {
    Waiting,
    Working(f32),
    Done(PathBuf),
    Skipped,
    Failed(String),
}

struct Item {
    path: PathBuf,
    state: State,
}

/// Sent from the worker thread: new state for the file at this index.
struct Update(usize, State);

struct Job {
    rx: Receiver<Update>,
    cancel: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

struct App {
    items: Vec<Item>,
    format: Format,
    out_dir: Option<PathBuf>,
    job: Option<Job>,
    ffmpeg_missing: Option<String>,
}

impl App {
    fn new() -> Self {
        App {
            items: Vec::new(),
            format: Format::Mp4,
            out_dir: None,
            job: None,
            ffmpeg_missing: convert::check_tools().err(),
        }
    }

    fn running(&self) -> bool {
        self.job.is_some()
    }

    /// Adds files, and the media files inside folders, skipping duplicates.
    fn add(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            let found = if path.is_dir() {
                let opts = Options {
                    inputs: vec![path],
                    recursive: true,
                    format: self.format,
                    ..Options::default()
                };
                convert::collect_files(&opts).unwrap_or_default()
            } else {
                vec![path]
            };
            for path in found {
                if !self.items.iter().any(|i| i.path == path) {
                    self.items.push(Item { path, state: State::Waiting });
                }
            }
        }
    }

    fn start(&mut self, ctx: &egui::Context) {
        let todo: Vec<(usize, PathBuf)> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| !matches!(i.state, State::Done(_)))
            .map(|(n, i)| (n, i.path.clone()))
            .collect();
        if todo.is_empty() {
            return;
        }
        for (n, _) in &todo {
            self.items[*n].state = State::Waiting;
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let opts = Options {
            format: self.format,
            out_dir: self.out_dir.clone(),
            cancel: Some(cancel.clone()),
            ..Options::default()
        };
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let thread = std::thread::spawn(move || {
            let send = |u: Update| {
                let _ = tx.send(u);
                ctx.request_repaint();
            };
            for (n, path) in todo {
                if opts.cancel.as_ref().unwrap().load(Ordering::Relaxed) {
                    break;
                }
                if let Some(dir) = &opts.out_dir {
                    let _ = std::fs::create_dir_all(dir);
                }
                send(Update(n, State::Working(0.0)));
                let result = convert::convert(&path, &opts, &mut |r| {
                    if let Report::Progress(pct) = r {
                        send(Update(n, State::Working(pct as f32 / 100.0)));
                    }
                });
                send(Update(
                    n,
                    match result {
                        Ok(Some(out)) => State::Done(out),
                        Ok(None) => State::Skipped,
                        Err(e) if e == "cancelled" => State::Waiting,
                        Err(e) => State::Failed(e),
                    },
                ));
            }
        });
        self.job = Some(Job { rx, cancel, thread });
    }

    fn stop(&mut self) {
        if let Some(job) = self.job.take() {
            job.cancel.store(true, Ordering::Relaxed);
            let _ = job.thread.join();
            for update in job.rx.try_iter() {
                self.items[update.0].state = update.1;
            }
        }
    }

    fn poll(&mut self) {
        let Some(job) = &self.job else { return };
        for Update(n, state) in job.rx.try_iter() {
            self.items[n].state = state;
        }
        if job.thread.is_finished() {
            self.stop(); // collects any last updates
        }
    }
}

impl Drop for App {
    /// Closing the window mid-conversion stops ffmpeg instead of leaving it running.
    fn drop(&mut self) {
        self.stop();
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();

        let (dropped, hovering) = ui.ctx().input(|i| {
            let dropped: Vec<PathBuf> =
                i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect();
            (dropped, !i.raw.hovered_files.is_empty())
        });
        if !dropped.is_empty() {
            self.add(dropped);
        }

        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(err) = &self.ffmpeg_missing {
                ui.colored_label(ui.visuals().error_fg_color, err);
                ui.add_space(6.0);
            }

            self.settings_row(ui);
            ui.add_space(8.0);

            egui::Panel::bottom("buttons")
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    ui.add_space(8.0);
                    self.buttons_row(ui);
                });

            egui::CentralPanel::default()
                .frame(egui::Frame::group(ui.style()))
                .show(ui, |ui| self.file_list(ui, hovering));
        });
    }
}

impl App {
    fn settings_row(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(!self.running(), |ui| {
            ui.horizontal(|ui| {
                ui.label("Convert to:");
                let before = self.format;
                egui::ComboBox::from_id_salt("format")
                    .selected_text(self.format.ext().to_uppercase())
                    .show_ui(ui, |ui| {
                        for (name, f) in FORMATS {
                            let label = if f.is_audio() {
                                format!("{}  (audio)", name.to_uppercase())
                            } else {
                                name.to_uppercase()
                            };
                            ui.selectable_value(&mut self.format, *f, label);
                        }
                    });
                if self.format != before {
                    // A new format means everything needs converting again.
                    for item in &mut self.items {
                        item.state = State::Waiting;
                    }
                }

                ui.add_space(16.0);
                ui.label("Save to:");
                let text = match &self.out_dir {
                    Some(dir) => short(dir),
                    None => "same folder as original".into(),
                };
                if ui.button(text).on_hover_text("Choose an output folder").clicked() {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        self.out_dir = Some(dir);
                    }
                }
                if self.out_dir.is_some()
                    && ui.small_button("x").on_hover_text("Save next to the originals").clicked()
                {
                    self.out_dir = None;
                }
            });
        });
    }

    fn buttons_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Add files...").clicked() {
                if let Some(files) = rfd::FileDialog::new().pick_files() {
                    self.add(files);
                }
            }
            let can_clear = !self.running() && !self.items.is_empty();
            if ui.add_enabled(can_clear, egui::Button::new("Clear")).clicked() {
                self.items.clear();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.running() {
                    if ui.button("Stop").clicked() {
                        self.stop();
                    }
                    ui.spinner();
                } else {
                    let todo = self.items.iter().filter(|i| !matches!(i.state, State::Done(_))).count();
                    let label = match todo {
                        0 => "Convert".to_string(),
                        1 => "Convert 1 file".to_string(),
                        n => format!("Convert {n} files"),
                    };
                    let ok = todo > 0 && self.ffmpeg_missing.is_none();
                    let button = egui::Button::new(egui::RichText::new(label).strong())
                        .min_size(egui::vec2(130.0, 28.0));
                    if ui.add_enabled(ok, button).clicked() {
                        self.start(ui.ctx());
                    }
                }
            });
        });
    }

    fn file_list(&mut self, ui: &mut egui::Ui, hovering: bool) {
        if self.items.is_empty() {
            let text = if hovering { "Drop to add" } else { "Drop files or folders here\n\nor click to choose files" };
            let response = ui.centered_and_justified(|ui| {
                ui.add(egui::Label::new(egui::RichText::new(text).size(16.0).weak()).sense(egui::Sense::click()))
            });
            if response.inner.clicked() {
                if let Some(files) = rfd::FileDialog::new().pick_files() {
                    self.add(files);
                }
            }
            return;
        }

        let running = self.running();
        let mut remove = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for (n, item) in self.items.iter().enumerate() {
                ui.horizontal(|ui| {
                    let name = item.path.file_name().unwrap_or_default().to_string_lossy();
                    ui.label(name.as_ref()).on_hover_text(item.path.display().to_string());

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !running && ui.small_button("x").on_hover_text("Remove from list").clicked() {
                            remove = Some(n);
                        }
                        match &item.state {
                            State::Waiting => {
                                ui.weak("waiting");
                            }
                            State::Working(p) => {
                                ui.add(egui::ProgressBar::new(*p).show_percentage().desired_width(140.0));
                            }
                            State::Done(out) => {
                                if ui.link("done - show").on_hover_text(out.display().to_string()).clicked() {
                                    show_in_folder(out);
                                }
                            }
                            State::Skipped => {
                                ui.weak("already exists").on_hover_text("A file with the new name is already there");
                            }
                            State::Failed(e) => {
                                ui.colored_label(ui.visuals().error_fg_color, "failed").on_hover_text(e);
                            }
                        }
                    });
                });
            }
        });
        if let Some(n) = remove {
            self.items.remove(n);
        }
    }
}

/// The folder name, or the full path if short enough.
fn short(dir: &Path) -> String {
    let full = dir.display().to_string();
    if full.chars().count() <= 32 {
        full
    } else {
        format!(".../{}", dir.file_name().unwrap_or_default().to_string_lossy())
    }
}

fn show_in_folder(file: &Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg("/select,").arg(file).spawn();
    }
    #[cfg(not(windows))]
    {
        let dir = file.parent().unwrap_or(Path::new("."));
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let _ = std::process::Command::new(opener).arg(dir).spawn();
    }
}
