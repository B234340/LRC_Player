use crate::application::dto::{LibraryDto, PlaylistDto, SettingsDto, StateDto};
use crate::application::usecases::build_app_vm::build_app_vm;
use crate::application::view_model::{AppVm, QueueVm};
use crate::config::{Config, Language};
use crate::infrastructure::audio;
use crate::localization::{language_name, supported_languages, translate};
use crate::ports::audio_output::{AudioFormat, SampleFormat};
use crate::presentation::tabs;
use crate::shared::ipc as ipc_mod;
use cpal::traits::{DeviceTrait, HostTrait};
use eframe::{egui, App, CreationContext};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Library,
    Play,
    Playlist,
    Album,
    Artist,
    Queue,
    Scan,
    Settings,
}

impl Tab {
    fn label(&self) -> &'static str {
        match self {
            Tab::Library => "Library",
            Tab::Play => "Play",
            Tab::Playlist => "Playlists",
            Tab::Album => "Albums",
            Tab::Artist => "Artists",
            Tab::Queue => "Queue",
            Tab::Scan => "Scan",
            Tab::Settings => "Settings",
        }
    }
}

pub struct GuiApp {
    app_vm: AppVm,
    active_tab: Tab,
    device_names: Vec<String>,
    selected: Option<usize>,
    device_filter: String,
    play_path: String,
    taps: usize,
    language: Language,
}

impl GuiApp {
    fn default_app_vm(active_tab: Tab, config: &Config) -> AppVm {
        let settings = SettingsDto::new(
            config.preferred_output_device.clone(),
            config.resampler_taps,
        );
        let state = StateDto {
            last_tab: Some(active_tab.label().to_string()),
        };
        let playlists: Vec<PlaylistDto> = Vec::new();
        let library = LibraryDto {
            album_count: 0,
            track_count: 0,
        };
        let queue = QueueVm::new(Vec::new());

        build_app_vm("LRC Player", &settings, &state, &playlists, &library, queue)
    }

    fn update_app_vm(&mut self) {
        let cfg = Config::load();
        self.language = cfg.language;
        self.app_vm = Self::default_app_vm(self.active_tab, &cfg);
    }

    pub fn new(_cc: &CreationContext<'_>) -> Self {
        let mut device_names = Vec::new();
        if let Ok(devs) = cpal::default_host().output_devices() {
            for d in devs {
                if let Ok(name) = d.name() {
                    device_names.push(name);
                }
            }
        }
        let cfg = Config::load();
        let selected = cfg
            .preferred_output_device
            .as_ref()
            .and_then(|n| device_names.iter().position(|s| s == n));
        let active_tab = Tab::Library;

        GuiApp {
            app_vm: Self::default_app_vm(active_tab, &cfg),
            active_tab,
            device_names,
            selected,
            device_filter: String::new(),
            play_path: String::new(),
            taps: cfg.resampler_taps,
            language: cfg.language,
        }
    }
}

impl App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.heading(translate("title", self.language));
            ui.horizontal(|ui| {
                for tab in [
                    Tab::Library,
                    Tab::Play,
                    Tab::Playlist,
                    Tab::Album,
                    Tab::Artist,
                    Tab::Queue,
                    Tab::Scan,
                    Tab::Settings,
                ] {
                    if ui
                        .selectable_label(self.active_tab == tab, tab.label())
                        .clicked()
                    {
                        self.active_tab = tab;
                        self.update_app_vm();
                    }
                }
            });
        });

        egui::SidePanel::left("side_panel").resizable(false).show(ctx, |ui| {
            ui.heading(translate("output_device", self.language));
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label(translate("filter", self.language));
                ui.text_edit_singleline(&mut self.device_filter);
            });

            let filter = self.device_filter.to_lowercase();
            let filtered: Vec<(usize, &String)> = if filter.is_empty() {
                self.device_names.iter().enumerate().collect()
            } else {
                self.device_names
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| n.to_lowercase().contains(&filter))
                    .collect()
            };

            if filtered.is_empty() {
                ui.label(translate("no_matching_devices", self.language));
            } else {
                let default_sel = self.selected.unwrap_or(0);
                let mut sel_in_filtered = 0usize;
                for (i, (orig_idx, _)) in filtered.iter().enumerate() {
                    if *orig_idx == default_sel {
                        sel_in_filtered = i;
                        break;
                    }
                }

                egui::ComboBox::from_id_source("device_combo")
                    .selected_text(filtered.get(sel_in_filtered).map(|(_, s)| s.as_str()).unwrap_or(""))
                    .show_ui(ui, |ui| {
                        for (i, (_orig_idx, name)) in filtered.iter().enumerate() {
                            ui.selectable_value(&mut sel_in_filtered, i, name.as_str());
                        }
                    });

                if let Some((orig_idx, _)) = filtered.get(sel_in_filtered) {
                    self.selected = Some(*orig_idx);
                }
            }

            if ui.button(translate("save_device", self.language)).clicked() {
                if let Some(idx) = self.selected {
                    let name = self.device_names[idx].clone();
                    let mut cfg = Config::load();
                    cfg.preferred_output_device = Some(name);
                    let _ = cfg.save();
                    self.update_app_vm();
                }
            }

            ui.add_space(8.0);
            ui.label(translate("resampler_taps", self.language));
            ui.add(egui::DragValue::new(&mut self.taps).clamp_range(4..=256));
            if ui.button(translate("save_taps", self.language)).clicked() {
                let mut cfg = Config::load();
                cfg.resampler_taps = self.taps;
                let _ = cfg.save();
                self.update_app_vm();
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(translate("language", self.language));
                let mut selected_lang = self.language;
                egui::ComboBox::from_id_source("language_combo")
                    .selected_text(language_name(self.language))
                    .show_ui(ui, |ui| {
                        for lang in supported_languages() {
                            ui.selectable_value(&mut selected_lang, *lang, language_name(*lang));
                        }
                    });

                if selected_lang != self.language {
                    self.language = selected_lang;
                }
            });

            if ui.button("Save language").clicked() {
                let mut cfg = Config::load();
                cfg.language = self.language;
                let _ = cfg.save();
                self.update_app_vm();
            }

            ui.separator();
            ui.label(translate("play_demo", self.language));
            ui.text_edit_singleline(&mut self.play_path);
            if ui.button(translate("send_scan", self.language)).clicked() {
                let path = self.play_path.clone();
                if !path.is_empty() {
                    let socket = ipc_mod::default_socket_path();
                    let request = ipc_mod::ScanRequest { paths: vec![path] };
                    std::thread::spawn(move || {
                        if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                        {
                            let _ = rt.block_on(async move {
                                ipc_mod::send_scan_request(&socket, &request).await
                            });
                        }
                    });
                }
            }
            if ui.button(translate("play", self.language)).clicked() {
                let path = std::path::PathBuf::from(self.play_path.clone());
                if path.exists() {
                    std::thread::spawn(move || {
                        let mut out = audio::CpalOutput::new();
                        let req_fmt = AudioFormat {
                            sample_rate: 44100,
                            channels: 2,
                            sample_format: SampleFormat::F32,
                        };
                        match out.start_with_device(req_fmt, Config::load().preferred_output_device.clone()) {
                            Ok(eff_fmt) => {
                                let producer = out.producer_sender().expect("no producer");
                                let pool = out.pool_handle();
                                let _ = crate::infrastructure::audio::decoder::spawn_decode_to_producer(
                                    path,
                                    1024usize,
                                    eff_fmt.sample_rate,
                                    eff_fmt.channels,
                                    producer,
                                    pool,
                                    Config::load().resampler_taps,
                                );
                                loop {
                                    std::thread::sleep(std::time::Duration::from_secs(60));
                                }
                            }
                            Err(e) => {
                                eprintln!("audio start failed: {}", e);
                            }
                        }
                    });
                }
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.active_tab {
            Tab::Library => tabs::library_tab::ui(ui, &self.app_vm),
            Tab::Play => tabs::play_tab::ui(ui, &self.app_vm),
            Tab::Playlist => tabs::playlist_tab::ui(ui, &self.app_vm),
            Tab::Album => tabs::album_tab::ui(ui, &self.app_vm),
            Tab::Artist => tabs::artist_tab::ui(ui, &self.app_vm),
            Tab::Queue => tabs::queue_tab::ui(ui, &self.app_vm),
            Tab::Scan => tabs::scan_tab::ui(ui, &self.app_vm),
            Tab::Settings => tabs::settings_tab::ui(ui, &self.app_vm),
        });
    }
}
