use crate::config::Config;
use crate::infrastructure::audio;
use crate::ports::audio_output::{AudioFormat, SampleFormat};
use cpal::traits::{DeviceTrait, HostTrait};
use eframe::{egui, App, CreationContext};

pub struct GuiApp {
    device_names: Vec<String>,
    selected: Option<usize>,
    device_filter: String,
    play_path: String,
    taps: usize,
}

impl GuiApp {
    pub fn new(_cc: &CreationContext<'_>) -> Self {
        let mut device_names = Vec::new();
        if let Ok(devs) = cpal::default_host().output_devices() {
            for d in devs {
                if let Ok(name) = d.name() {
                    device_names.push(name);
                }
            }
        }
        // load saved config
        let cfg = Config::load();
        let selected = cfg
            .preferred_output_device
            .as_ref()
            .and_then(|n| device_names.iter().position(|s| s == n));

        GuiApp {
            device_names,
            selected,
            device_filter: String::new(),
            play_path: String::new(),
            taps: cfg.resampler_taps,
        }
    }
}

impl App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("LRC Player — Minimal GUI");
            ui.add_space(8.0);

            ui.group(|ui| {
                ui.label("Output device:");
                ui.horizontal(|ui| {
                    ui.label("Filter:");
                    ui.text_edit_singleline(&mut self.device_filter);
                });
                if self.device_names.is_empty() {
                    ui.label("(no devices found)");
                } else {
                    // compute filtered list indices
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
                        ui.label("(no devices match filter)");
                    } else {
                        // map current selection to index within filtered list
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

                        // update self.selected to original index of picked device
                        if let Some((orig_idx, _)) = filtered.get(sel_in_filtered) {
                            self.selected = Some(*orig_idx);
                        }
                    }
                    if ui.button("Save device").clicked() {
                        if let Some(idx) = self.selected {
                            let name = self.device_names[idx].clone();
                            let mut cfg = Config::load();
                            cfg.preferred_output_device = Some(name);
                            let _ = cfg.save();
                        }
                    }
                }
            });

            ui.add_space(8.0);
            ui.group(|ui| {
                ui.label("Resampler taps:");
                ui.add(egui::DragValue::new(&mut self.taps).clamp_range(4..=256));
                if ui.button("Save taps").clicked() {
                    let mut cfg = Config::load();
                    cfg.resampler_taps = self.taps;
                    let _ = cfg.save();
                }
            });

            ui.add_space(8.0);
            ui.group(|ui| {
                ui.label("Play demo (enter path):");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.play_path);
                    if ui.button("Play").clicked() {
                        // launch decoder in background using existing library API
                        let path = std::path::PathBuf::from(self.play_path.clone());
                        if path.exists() {
                            // start output and decoder in a thread; best-effort demo
                            std::thread::spawn(move || {
                                let mut out = audio::CpalOutput::new();
                                let req_fmt = AudioFormat { sample_rate: 44100, channels: 2, sample_format: SampleFormat::F32 };
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
                                        loop { std::thread::sleep(std::time::Duration::from_secs(60)); }
                                    }
                                    Err(e) => {
                                        eprintln!("audio start failed: {}", e);
                                    }
                                }
                            });
                        }
                    }
                });
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Scan (enter path below)").clicked() {
                    // placeholder — scanning is managed by ScanCoordinator in library
                }
            });
            ui.text_edit_singleline(&mut self.play_path);

            ui.add_space(12.0);
            ui.label("This is an early prototype GUI. Use CLI for robust control.");
        });
    }
}
