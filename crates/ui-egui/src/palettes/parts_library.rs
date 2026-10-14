//! Parts Library palette — browse folders of DXF/DWG symbols as a thumbnail grid and insert them.
//! Libraries are copied into the CADCraft settings directory so they survive deletion of the
//! original folder.

use std::path::PathBuf;
use std::sync::mpsc;

use cadcraft_engine::cmd::library::{self, LibraryFolder};
use egui::vec2;
use serde_json::{Value, json};

use crate::CadApp;
use crate::docking::Panel;
use crate::theme::Tokens;

const THUMB_W: u32 = 96;
const THUMB_H: u32 = 72;
const CELL_W: f32 = 100.0;
const CELL_H: f32 = 100.0;
const CACHE_CAP: usize = 200;

pub struct PartsLibraryState {
    pub trees: Vec<LibraryFolder>,
    pub selected: Option<PathBuf>,
    pub selected_library: Option<String>,
    pub selected_folder: Option<PathBuf>,
    cache: Vec<(PathBuf, egui::TextureHandle)>,
    pending: Vec<PathBuf>,
    inflight: usize,
    tx: mpsc::Sender<(PathBuf, egui::ColorImage)>,
    rx: mpsc::Receiver<(PathBuf, egui::ColorImage)>,
    dirty: bool,
}

impl Default for PartsLibraryState {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            trees: Vec::new(),
            selected: None,
            selected_library: None,
            selected_folder: None,
            cache: Vec::new(),
            pending: Vec::new(),
            inflight: 0,
            tx,
            rx,
            dirty: true,
        }
    }
}

fn libraries_dir(app: &CadApp) -> Option<PathBuf> {
    app.services.config_dir.as_ref().map(|d| d.join("libraries"))
}

pub fn open(app: &mut CadApp) -> Result<Value, String> {
    let panel = Panel::PartsLibrary;
    let first = !app.ui.docking.layout.contains(&panel) && !app.ui.docking.hidden.iter().any(|(p, _)| *p == panel);
    let r = crate::docking::command(app, false, &json!({ "operation": "open", "panel": panel.id() }))?;
    if first && app.ui.docking.layout.contains(&Panel::ToolSets) {
        let moved = json!({ "operation": "move", "panel": panel.id(), "target": Panel::ToolSets.id(), "zone": "bottom" });
        if let Ok(r) = crate::docking::command(app, false, &moved) {
            return Ok(r);
        }
    }
    Ok(r)
}

pub fn show(app: &mut CadApp, ui: &mut egui::Ui) {
    let t = Tokens::get();

    #[cfg(not(target_arch = "wasm32"))]
    {
        collect_previews(app, ui.ctx());

        let lib_dir = libraries_dir(app);
        if app.parts_library.dirty {
            app.parts_library.trees.clear();
            if let Some(ref dir) = lib_dir {
                let _ = std::fs::create_dir_all(dir);
                if let Ok(names) = library::list_libraries(dir) {
                    for name in names {
                        if let Ok(tree) = library::scan_library(&dir.join(&name)) {
                            app.parts_library.trees.push(tree);
                        }
                    }
                }
            }
            app.parts_library.dirty = false;
        }

        ui.add_space(4.0);
        ui.spacing_mut().item_spacing = vec2(4.0, 2.0);

        toolbar(app, ui, &lib_dir);
        ui.add_space(4.0);
        ui.separator();

        if app.parts_library.trees.is_empty() {
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(crate::i18n::t("No libraries installed")).color(t.text_dim));
                ui.add_space(4.0);
                ui.label(egui::RichText::new(crate::i18n::t("Click + Add to import a folder of DXF/DWG symbols")).color(t.text_dim).small());
            });
        } else {
            let tree_height = (ui.available_height() * 0.35).clamp(60.0, 200.0);

            egui::ScrollArea::vertical().id_salt("parts-tree").auto_shrink([false, false]).max_height(tree_height).show(ui, |ui| {
                let trees: Vec<LibraryFolder> = app.parts_library.trees.clone();
                for tree in &trees {
                    show_folder_tree(ui, tree, &mut app.parts_library.selected_folder, &mut app.parts_library.selected_library);
                }
            });

            ui.separator();

            thumbnail_grid(app, ui, t);
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("Parts Library is not available on the web").color(t.text_dim));
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn toolbar(app: &mut CadApp, ui: &mut egui::Ui, lib_dir: &Option<PathBuf>) {
    let t = Tokens::get();
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        if ui.button(egui::RichText::new(format!("+ {}", crate::i18n::t("Add"))).color(t.accent)).clicked() {
            if let Some(ref picker) = app.services.pick_folder {
                if let (Some(picked), Some(dir)) = (picker(), lib_dir) {
                    let source = PathBuf::from(&picked);
                    match library::import_library(&source, dir) {
                        Ok(_) => {
                            app.set_status(format!("Library imported: {picked}"));
                            app.parts_library.dirty = true;
                        }
                        Err(e) => app.set_status(format!("Import failed: {e}")),
                    }
                }
            } else {
                app.set_status("Folder picker not available".to_string());
            }
        }

        let can_remove = app.parts_library.selected_library.is_some();
        let remove_btn = ui.add_enabled(
            can_remove,
            egui::Button::new(egui::RichText::new(crate::i18n::t("Remove")).color(if can_remove { t.text } else { t.text_dim })),
        );
        if remove_btn.clicked()
            && let (Some(name), Some(dir)) = (&app.parts_library.selected_library, lib_dir)
        {
            match library::remove_library(name, dir) {
                Ok(()) => {
                    app.set_status(format!("Library removed: {name}"));
                    app.parts_library.selected = None;
                    app.parts_library.selected_library = None;
                    app.parts_library.selected_folder = None;
                    app.parts_library.dirty = true;
                }
                Err(e) => app.set_status(format!("Remove failed: {e}")),
            }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn show_folder_tree(ui: &mut egui::Ui, folder: &LibraryFolder, selected_folder: &mut Option<PathBuf>, selected_library: &mut Option<String>) {
    let has_children = !folder.children.is_empty();
    let has_entries = !folder.entries.is_empty();
    let is_leaf = !has_children && has_entries;

    if is_leaf {
        let is_selected = selected_folder.as_ref().is_some_and(|s| *s == folder.path);
        let response = ui.selectable_label(is_selected, &folder.name);
        if response.clicked() {
            *selected_folder = Some(folder.path.clone());
            *selected_library = root_library_name(&folder.path);
        }
    } else {
        let is_selected = selected_folder.as_ref().is_some_and(|s| *s == folder.path);
        let header = egui::CollapsingHeader::new(egui::RichText::new(&folder.name).strong().color(if is_selected {
            Tokens::get().accent
        } else {
            Tokens::get().text
        }))
        .id_salt(&folder.path)
        .default_open(false);

        header.show(ui, |ui| {
            if has_entries {
                let self_label = ui.selectable_label(is_selected, egui::RichText::new("(this folder)").italics().small());
                if self_label.clicked() {
                    *selected_folder = Some(folder.path.clone());
                    *selected_library = root_library_name(&folder.path);
                }
            }
            for child in &folder.children {
                show_folder_tree(ui, child, selected_folder, selected_library);
            }
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn root_library_name(path: &std::path::Path) -> Option<String> {
    for ancestor in path.ancestors() {
        if let Some(parent) = ancestor.parent()
            && parent.file_name().and_then(|n| n.to_str()) == Some("libraries")
        {
            return ancestor.file_name().and_then(|n| n.to_str()).map(String::from);
        }
    }
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn find_folder<'a>(trees: &'a [LibraryFolder], path: &std::path::Path) -> Option<&'a LibraryFolder> {
    for tree in trees {
        if tree.path == path {
            return Some(tree);
        }
        if let Some(found) = find_folder(&tree.children, path) {
            return Some(found);
        }
    }
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn thumbnail_grid(app: &mut CadApp, ui: &mut egui::Ui, t: Tokens) {
    let folder_path = match app.parts_library.selected_folder.clone() {
        Some(p) => p,
        None => {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(crate::i18n::t("Select a folder to browse symbols")).color(t.text_dim).small());
            });
            return;
        }
    };

    let entries: Vec<_> = find_folder(&app.parts_library.trees, &folder_path).map(|f| f.entries.clone()).unwrap_or_default();

    if entries.is_empty() {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(crate::i18n::t("No symbols in this folder")).color(t.text_dim).small());
        });
        return;
    }

    queue_missing_thumbnails(app, &entries);

    let mut insert_path: Option<PathBuf> = None;

    egui::ScrollArea::vertical().id_salt("parts-grid").auto_shrink([false, false]).show(ui, |ui| {
        let avail_w = ui.available_width();
        let cols = ((avail_w / CELL_W) as usize).max(1);

        egui::Grid::new("thumb-grid").num_columns(cols).spacing(vec2(4.0, 6.0)).show(ui, |ui| {
            for (i, entry) in entries.iter().enumerate() {
                let is_selected = app.parts_library.selected.as_ref().is_some_and(|s| *s == entry.path);

                let response = ui.allocate_ui(vec2(CELL_W, CELL_H), |ui| {
                    let cached = app.parts_library.cache.iter().find(|(p, _)| *p == entry.path).map(|(_, tex)| tex);

                    if let Some(tex) = cached {
                        let img_response = ui.add(egui::Image::new(tex).fit_to_exact_size(vec2(THUMB_W as f32, THUMB_H as f32)));
                        if is_selected {
                            ui.painter().rect_stroke(img_response.rect, 2.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Outside);
                        }
                    } else {
                        let (rect, _) = ui.allocate_exact_size(vec2(THUMB_W as f32, THUMB_H as f32), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 2.0, egui::Color32::from_gray(60));
                        ui.put(rect, egui::Spinner::new().size(16.0));
                    }

                    ui.label(egui::RichText::new(&entry.name).small().color(if is_selected { t.accent } else { t.text_dim }));
                });

                if response.response.interact(egui::Sense::click()).clicked() {
                    app.parts_library.selected = Some(entry.path.clone());
                }
                if response.response.interact(egui::Sense::click()).double_clicked() {
                    insert_path = Some(entry.path.clone());
                }

                if (i + 1) % cols == 0 {
                    ui.end_row();
                }
            }
        });
    });

    if let Some(path) = insert_path {
        do_insert(app, &path);
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        let can_insert = app.parts_library.selected.is_some();
        if ui.add_enabled(can_insert, egui::Button::new(crate::i18n::t("Insert"))).clicked()
            && let Some(ref sel) = app.parts_library.selected.clone()
        {
            do_insert(app, sel);
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn queue_missing_thumbnails(app: &mut CadApp, entries: &[library::LibraryEntry]) {
    for entry in entries {
        let cached = app.parts_library.cache.iter().any(|(p, _)| *p == entry.path);
        let queued = app.parts_library.pending.contains(&entry.path);
        if !cached && !queued {
            app.parts_library.pending.push(entry.path.clone());
        }
    }
    flush_pending(app);
}

#[cfg(not(target_arch = "wasm32"))]
fn flush_pending(app: &mut CadApp) {
    while app.parts_library.inflight < 4 {
        let path = match app.parts_library.pending.pop() {
            Some(p) => p,
            None => break,
        };
        if app.parts_library.cache.iter().any(|(p, _)| *p == path) {
            continue;
        }
        app.parts_library.inflight += 1;
        let tx = app.parts_library.tx.clone();
        std::thread::spawn(move || {
            let img = render_thumbnail(&path).unwrap_or_else(|| {
                egui::ColorImage::from_rgba_unmultiplied([THUMB_W as usize, THUMB_H as usize], &vec![200; (THUMB_W * THUMB_H * 4) as usize])
            });
            let _ = tx.send((path, img));
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn render_thumbnail(path: &std::path::Path) -> Option<egui::ColorImage> {
    let bytes = std::fs::read(path).ok()?;
    let name = path.to_string_lossy();
    let hooks = cadcraft_engine::cmd::file::io()?;
    let drawing = (hooks.read)(&bytes, &name).ok()?;

    let opts = cadcraft_render::Options { transparency: true, ..Default::default() };
    let list = cadcraft_render::build(&drawing, &cadcraft_doc::Space::Model, &opts);
    if list.bounds.is_empty() {
        return None;
    }

    let view = cadcraft_render::raster::View::fit(&list.bounds, THUMB_W, THUMB_H, 0.08);
    let ropts = cadcraft_render::raster::RasterOptions { background: cadcraft_color::Rgb(240, 240, 240), ..Default::default() };
    let pixmap = cadcraft_render::raster::render(&list, &view, &ropts)?;
    let size = [pixmap.width() as usize, pixmap.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, pixmap.data()))
}

#[cfg(not(target_arch = "wasm32"))]
fn collect_previews(app: &mut CadApp, ctx: &egui::Context) {
    let mut got_any = false;
    while let Ok((path, image)) = app.parts_library.rx.try_recv() {
        let tex = ctx.load_texture(format!("parts-thumb-{}", path.display()), image, egui::TextureOptions::LINEAR);
        if app.parts_library.cache.len() >= CACHE_CAP {
            drop(app.parts_library.cache.remove(0));
        }
        app.parts_library.cache.push((path, tex));
        app.parts_library.inflight = app.parts_library.inflight.saturating_sub(1);
        got_any = true;
    }
    if got_any {
        flush_pending(app);
        ctx.request_repaint();
    }
}

fn do_insert(app: &mut CadApp, path: &std::path::Path) {
    let path_str = path.to_string_lossy().to_string();
    if let Err(e) = app.session.start("insert") {
        app.set_status(format!("Insert failed: {e}"));
        return;
    }
    app.cmdline(&path_str);
}
