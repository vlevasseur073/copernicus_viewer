//! Automated README screenshots (`COPERNICUS_VIEWER_CAPTURE_DEMO=<output-dir>`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, ColorImage, Event, UserData};
use image::{ImageBuffer, RgbaImage};

use copernicus_viewer::comparison::ComparisonTool;
use copernicus_viewer::plot::PlotPanel;
use copernicus_viewer::zarr::ZarrStore;

const LST_PATH: &str = "/measurements/lst";
const EXPLORE_SHOT: &str = "01-explore-lst.png";
const COMPARE_SHOT: &str = "02-comparison.png";
const FRAMES_BEFORE_SHOT: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    WaitStores,
    WaitPlot,
    WaitFrames,
    RequestExploreShot,
    RunComparison,
    WaitFramesCompare,
    RequestCompareShot,
    Finished,
}

pub enum DemoAction {
    SelectLst,
    RunComparison,
    Close,
}

pub struct DemoCapture {
    out_dir: PathBuf,
    step: Step,
    frames_left: u32,
    pending_shot: Option<String>,
    shots_saved: usize,
}

impl DemoCapture {
    pub fn from_env() -> Option<Self> {
        let out_dir = std::env::var("COPERNICUS_VIEWER_CAPTURE_DEMO")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)?;
        Some(Self {
            out_dir,
            step: Step::WaitStores,
            frames_left: 0,
            pending_shot: None,
            shots_saved: 0,
        })
    }

    pub fn tick(
        &mut self,
        ctx: &egui::Context,
        stores: &[Arc<ZarrStore>],
        plot_panel: &PlotPanel,
        comparison: &ComparisonTool,
    ) -> Option<DemoAction> {
        if self.step == Step::Finished {
            return None;
        }

        match self.step {
            Step::WaitStores if stores.len() >= 2 => {
                if stores[0].tree.root.find_by_path(LST_PATH).is_some() {
                    self.step = Step::WaitPlot;
                    return Some(DemoAction::SelectLst);
                }
                eprintln!("Demo capture: variable {LST_PATH} not found in first product");
                self.finish(ctx);
                None
            }
            Step::WaitPlot if plot_panel.is_plot_ready() => {
                self.frames_left = FRAMES_BEFORE_SHOT;
                self.step = Step::WaitFrames;
                None
            }
            Step::WaitFrames => {
                if self.frames_left == 0 {
                    self.request_shot(ctx, EXPLORE_SHOT);
                    self.step = Step::RequestExploreShot;
                } else {
                    self.frames_left -= 1;
                }
                None
            }
            Step::RequestExploreShot if self.pending_shot.is_none() => {
                self.step = Step::RunComparison;
                Some(DemoAction::RunComparison)
            }
            Step::RunComparison if comparison.has_result() => {
                self.frames_left = FRAMES_BEFORE_SHOT;
                self.step = Step::WaitFramesCompare;
                None
            }
            Step::WaitFramesCompare => {
                if self.frames_left == 0 {
                    self.request_shot(ctx, COMPARE_SHOT);
                    self.step = Step::RequestCompareShot;
                } else {
                    self.frames_left -= 1;
                }
                None
            }
            Step::RequestCompareShot if self.pending_shot.is_none() => {
                self.finish(ctx);
                Some(DemoAction::Close)
            }
            _ => None,
        }
    }

    pub fn handle_events(&mut self, ctx: &egui::Context) {
        let events: Vec<Event> = ctx.input(|input| input.events.clone());
        for event in events {
            let Event::Screenshot {
                user_data, image, ..
            } = event
            else {
                continue;
            };
            let Some(filename) = user_data
                .data
                .as_ref()
                .and_then(|data| data.downcast_ref::<String>())
                .cloned()
            else {
                continue;
            };
            if self.pending_shot.as_deref() != Some(filename.as_str()) {
                continue;
            }

            let path = self.out_dir.join(&filename);
            if let Err(err) = save_png(&path, &image) {
                eprintln!("Demo capture: failed to write {}: {err}", path.display());
            } else {
                eprintln!("Demo capture: wrote {}", path.display());
                self.shots_saved += 1;
            }
            self.pending_shot = None;
        }
    }

    fn request_shot(&mut self, ctx: &egui::Context, filename: &str) {
        self.pending_shot = Some(filename.to_string());
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(UserData::new(
            filename.to_string(),
        )));
        ctx.request_repaint();
    }

    fn finish(&mut self, ctx: &egui::Context) {
        self.step = Step::Finished;
        eprintln!(
            "Demo capture: finished ({} screenshot{})",
            self.shots_saved,
            if self.shots_saved == 1 { "" } else { "s" }
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

fn save_png(path: &Path, image: &ColorImage) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let width = image.width() as u32;
    let height = image.height() as u32;
    let mut buffer: RgbaImage = ImageBuffer::new(width, height);
    for (pixel, color) in buffer.pixels_mut().zip(image.pixels.iter()) {
        *pixel = image::Rgba([color.r(), color.g(), color.b(), color.a()]);
    }
    buffer
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(|err| err.to_string())
}
