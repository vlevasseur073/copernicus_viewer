//! Dialog for editing the default `s3.conf` bucket credentials file.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use eframe::egui;

use copernicus_viewer::zarr::{S3BucketEntry, S3Config, clear_s3_client_cache, s3_config_path};

pub struct S3ConfigDialog {
    open: bool,
    config_path: PathBuf,
    using_env_override: bool,
    buckets: Vec<S3BucketEntry>,
    selected: usize,
    feedback: Option<(bool, String)>,
    save_error: Option<String>,
    test_in_progress: bool,
    test_request_id: u64,
    test_rx: Receiver<(u64, Result<(), String>)>,
    test_tx: Sender<(u64, Result<(), String>)>,
    saved: bool,
}

impl Default for S3ConfigDialog {
    fn default() -> Self {
        let (test_tx, test_rx) = mpsc::channel();
        Self {
            open: false,
            config_path: PathBuf::new(),
            using_env_override: false,
            buckets: Vec::new(),
            selected: 0,
            feedback: None,
            save_error: None,
            test_in_progress: false,
            test_request_id: 0,
            test_rx,
            test_tx,
            saved: false,
        }
    }
}

impl S3ConfigDialog {
    pub fn show(&mut self) {
        self.open = true;
        self.saved = false;
        self.feedback = None;
        self.save_error = None;
        self.test_in_progress = false;
        self.test_result_ready();

        self.using_env_override = s3_config_path().is_some();
        self.config_path = S3Config::effective_s3_config_path().unwrap_or_default();

        self.buckets = S3Config::load_bucket_entries(&self.config_path)
            .map_err(|e| e.to_string())
            .unwrap_or_else(|err| {
                self.feedback = Some((false, err));
                Vec::new()
            });

        if self.buckets.is_empty() {
            self.buckets.push(empty_bucket_entry());
        }
        self.selected = 0;
    }

    pub fn take_saved(&mut self) -> bool {
        let saved = self.saved;
        self.saved = false;
        saved
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }

        self.test_result_ready();

        let (default_size, max_size) = dialog_viewport_sizes(ctx);

        let mut keep_open = self.open;
        let mut dismiss = false;
        egui::Window::new("S3 configuration")
            .collapsible(false)
            .resizable(true)
            .default_size(default_size)
            .max_size(max_size)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut keep_open)
            .show(ctx, |ui| {
                ui.set_width(ui.available_width());

                ui.label(
                    "Configure S3 bucket credentials used by the open-product browser \
                     and s3:// URIs.",
                );
                ui.separator();

                if self.config_path.as_os_str().is_empty() {
                    ui.colored_label(
                        egui::Color32::LIGHT_RED,
                        "Cannot determine a config file path on this system.",
                    );
                } else {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Config file:");
                        ui.monospace(self.config_path.display().to_string());
                    });
                }

                if self.using_env_override {
                    ui.label(
                        egui::RichText::new(
                            "COPERNICUS_VIEWER_S3_CONFIG or S3_CONFIG is set — \
                             edits are saved to that file.",
                        )
                        .small()
                        .weak(),
                    );
                }

                if let Some((ok, message)) = &self.feedback {
                    let color = if *ok {
                        egui::Color32::LIGHT_GREEN
                    } else {
                        egui::Color32::LIGHT_RED
                    };
                    ui.colored_label(color, message);
                }

                if let Some(err) = &self.save_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, err);
                }

                ui.add_space(8.0);
                let body_height = (ui.available_height() - 44.0).max(160.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(body_height)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.columns(2, |columns| {
                            let [list, form] = columns else {
                                return;
                            };

                            list.label(egui::RichText::new("Buckets").strong());
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .max_height((body_height - 36.0).max(120.0))
                                .show(list, |ui| {
                                    let mut select = None;
                                    for (index, entry) in self.buckets.iter().enumerate() {
                                        let label = if entry.bucket.trim().is_empty() {
                                            "(new bucket)".to_string()
                                        } else {
                                            entry.bucket.clone()
                                        };
                                        if ui
                                            .selectable_label(index == self.selected, label)
                                            .clicked()
                                        {
                                            select = Some(index);
                                        }
                                    }
                                    if let Some(index) = select {
                                        self.selected = index;
                                        self.feedback = None;
                                        self.save_error = None;
                                    }
                                });

                            list.horizontal(|ui| {
                                if ui.button("Add").clicked() {
                                    self.buckets.push(empty_bucket_entry());
                                    self.selected = self.buckets.len() - 1;
                                    self.feedback = None;
                                    self.save_error = None;
                                }
                                let can_remove = self.buckets.len() > 1;
                                ui.add_enabled_ui(can_remove, |ui| {
                                    if ui.button("Remove").clicked() {
                                        self.buckets.remove(self.selected);
                                        self.selected = self.selected.min(self.buckets.len() - 1);
                                        self.feedback = None;
                                        self.save_error = None;
                                    }
                                });
                            });

                            if let Some(entry) = self.buckets.get_mut(self.selected) {
                                bucket_form_ui(form, entry);
                            }
                        });
                    });

                ui.add_space(8.0);
                ui.separator();
                ui.horizontal(|ui| {
                    let can_test = self
                        .buckets
                        .get(self.selected)
                        .is_some_and(|entry| validate_bucket_entry(entry).is_ok())
                        && !self.test_in_progress;
                    ui.add_enabled_ui(can_test, |ui| {
                        if ui.button("Test connection").clicked() {
                            self.start_connection_test();
                        }
                    });
                    if self.test_in_progress {
                        ui.label(egui::RichText::new("Testing…").weak());
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let save_validation = validate_for_save(&self.buckets);
                        let can_save =
                            !self.config_path.as_os_str().is_empty() && save_validation.is_ok();
                        ui.add_enabled_ui(can_save, |ui| {
                            if ui.button("Save").clicked() {
                                self.save();
                                if self.saved {
                                    dismiss = true;
                                }
                            }
                        });
                        if ui.button("Cancel").clicked() {
                            dismiss = true;
                        }
                    });
                });

                if self.config_path.as_os_str().is_empty() {
                    ui.label(
                        egui::RichText::new("Save is disabled: cannot resolve a config file path.")
                            .small()
                            .weak(),
                    );
                } else if let Err(reason) = validate_for_save(&self.buckets) {
                    ui.label(
                        egui::RichText::new(format!("Save is disabled: {reason}"))
                            .small()
                            .weak(),
                    );
                }
            });

        self.open = keep_open && !dismiss;
    }

    fn test_result_ready(&mut self) {
        while let Ok((request_id, result)) = self.test_rx.try_recv() {
            if request_id != self.test_request_id {
                continue;
            }
            self.test_in_progress = false;
            self.feedback = Some(match result {
                Ok(()) => (true, "Connection successful.".to_string()),
                Err(err) => (false, err),
            });
        }
    }

    fn start_connection_test(&mut self) {
        let Some(entry) = self.buckets.get(self.selected).cloned() else {
            return;
        };
        if let Err(err) = validate_bucket_entry(&entry) {
            self.feedback = Some((false, err));
            return;
        }

        self.test_request_id += 1;
        let request_id = self.test_request_id;
        self.test_in_progress = true;
        self.feedback = None;
        let tx = self.test_tx.clone();
        thread::spawn(move || {
            let result = crate::s3_browser::test_bucket_connection(&entry);
            let _ = tx.send((request_id, result));
        });
    }

    fn save(&mut self) {
        self.save_error = None;
        self.feedback = None;

        let normalized = match validate_for_save(&self.buckets) {
            Ok(entries) => entries,
            Err(err) => {
                self.save_error = Some(err);
                return;
            }
        };

        match S3Config::save_bucket_entries(&self.config_path, &normalized) {
            Ok(()) => {
                clear_s3_client_cache();
                self.buckets = normalized;
                self.saved = true;
            }
            Err(err) => {
                self.save_error = Some(err.to_string());
            }
        }
    }
}

fn bucket_form_ui(ui: &mut egui::Ui, entry: &mut S3BucketEntry) {
    ui.label(egui::RichText::new("Bucket settings").strong());
    ui.add_space(4.0);

    let field_width = ui.available_width();
    bucket_field(
        ui,
        "Bucket name:",
        &mut entry.bucket,
        field_width,
        "my-eopf-bucket",
        false,
    );
    bucket_field(
        ui,
        "Access key ID:",
        &mut entry.access_key_id,
        field_width,
        "",
        false,
    );
    bucket_field(
        ui,
        "Secret access key:",
        &mut entry.secret_access_key,
        field_width,
        "",
        true,
    );
    bucket_field(
        ui,
        "Region:",
        &mut entry.region,
        field_width,
        "eu-west-1",
        false,
    );
    bucket_field(
        ui,
        "Endpoint:",
        &mut entry.endpoint,
        field_width,
        "https://s3.example.com",
        false,
    );
}

fn bucket_field(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    width: f32,
    hint: &str,
    password: bool,
) {
    ui.label(label);
    let mut edit = egui::TextEdit::singleline(value).desired_width(width);
    if !hint.is_empty() {
        edit = edit.hint_text(hint);
    }
    if password {
        edit = edit.password(true);
    }
    ui.add(edit);
    ui.add_space(4.0);
}

fn dialog_viewport_sizes(ctx: &egui::Context) -> (egui::Vec2, egui::Vec2) {
    // Read viewport size in a single `input` borrow — never call other `Context`
    // methods (e.g. `content_rect`) from inside that closure (egui deadlock).
    let viewport = ctx.input(|input| {
        input
            .viewport()
            .inner_rect
            .unwrap_or_else(|| input.content_rect())
    });
    let margin = 48.0;
    let max_size = egui::vec2(
        (viewport.width() - margin).clamp(360.0, viewport.width()),
        (viewport.height() - margin).clamp(280.0, viewport.height()),
    );
    let default_size = egui::vec2(640.0_f32.min(max_size.x), 480.0_f32.min(max_size.y));
    (default_size, max_size)
}

fn empty_bucket_entry() -> S3BucketEntry {
    S3BucketEntry {
        bucket: String::new(),
        access_key_id: String::new(),
        secret_access_key: String::new(),
        region: "eu-west-1".to_string(),
        endpoint: String::new(),
    }
}

fn normalize_bucket_entry(entry: &S3BucketEntry) -> S3BucketEntry {
    S3BucketEntry {
        bucket: entry.bucket.trim().to_string(),
        access_key_id: entry.access_key_id.trim().to_string(),
        secret_access_key: entry.secret_access_key.trim().to_string(),
        region: entry.region.trim().to_string(),
        endpoint: entry.endpoint.trim().to_string(),
    }
}

fn validate_bucket_entry(entry: &S3BucketEntry) -> Result<(), String> {
    let entry = normalize_bucket_entry(entry);
    if entry.bucket.is_empty() {
        return Err("Bucket name is required.".to_string());
    }
    if entry.access_key_id.is_empty() {
        return Err("Access key ID is required.".to_string());
    }
    if entry.secret_access_key.is_empty() {
        return Err("Secret access key is required.".to_string());
    }
    if entry.region.is_empty() {
        return Err("Region is required.".to_string());
    }
    if entry.endpoint.is_empty() {
        return Err("Endpoint is required.".to_string());
    }
    Ok(())
}

fn validate_all_entries(entries: &[S3BucketEntry]) -> Result<(), String> {
    if entries.is_empty() {
        return Err("Add at least one bucket.".to_string());
    }

    let mut seen = std::collections::HashSet::new();
    for entry in entries {
        validate_bucket_entry(entry)?;
        let name = entry.bucket.trim();
        if !seen.insert(name.to_string()) {
            return Err(format!("Duplicate bucket name: {name}"));
        }
    }
    Ok(())
}

fn is_placeholder_entry(entry: &S3BucketEntry) -> bool {
    let entry = normalize_bucket_entry(entry);
    entry.bucket.is_empty()
        && entry.access_key_id.is_empty()
        && entry.secret_access_key.is_empty()
        && entry.endpoint.is_empty()
}

fn entries_to_save(entries: &[S3BucketEntry]) -> Vec<S3BucketEntry> {
    entries
        .iter()
        .filter(|entry| !is_placeholder_entry(entry))
        .map(normalize_bucket_entry)
        .collect()
}

fn validate_for_save(entries: &[S3BucketEntry]) -> Result<Vec<S3BucketEntry>, String> {
    let to_save = entries_to_save(entries);
    if to_save.is_empty() {
        return Err("add at least one bucket with credentials".to_string());
    }
    validate_all_entries(&to_save)?;
    Ok(to_save)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_ignored_for_save_validation() {
        let entries = vec![
            S3BucketEntry {
                bucket: "bucket-a".to_string(),
                access_key_id: "AK".to_string(),
                secret_access_key: "SK".to_string(),
                region: "eu-west-1".to_string(),
                endpoint: "https://s3.example.com".to_string(),
            },
            empty_bucket_entry(),
        ];

        let saved = validate_for_save(&entries).expect("complete bucket should be savable");
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].bucket, "bucket-a");
    }

    #[test]
    fn partial_entries_still_block_save() {
        let entries = vec![S3BucketEntry {
            bucket: "bucket-a".to_string(),
            access_key_id: "AK".to_string(),
            secret_access_key: String::new(),
            region: "eu-west-1".to_string(),
            endpoint: "https://s3.example.com".to_string(),
        }];

        assert!(validate_for_save(&entries).is_err());
    }
}
