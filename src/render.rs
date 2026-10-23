//! Felis owns terminal painting; this adapter batches its display list for egui.
use std::{collections::HashMap, num::NonZeroU32, sync::Arc};

use eframe::egui::{
    self, Color32, ColorImage, Mesh, Pos2, Rect, Shape, TextureHandle, TextureOptions, Vec2,
};
use felis_client_core::ShadowScreen;
use felis_render_core::{
    SelectionRange,
    appearance::{FrameState, RowCells, RowFrame},
    display_list::{AtlasKind, DecoKind, RowPaint},
    glyphs::{GLYPH_ATLAS_SIDE, GlyphIndex, GridWalker, ShapeFrame, color_atlas_side},
    painter::CellPainter,
    palette::{ResolvedTheme, Theme, linear_rgba_to_srgb8},
};
use felis_shaping::{FontStack, GlyphPixels, Shaper, StyleFaces};

/// Discover once per GUI, including the font supplied by the Nix wrapper.
pub fn load_fonts(ctx: &egui::Context) -> anyhow::Result<Arc<FontStack>> {
    if let Some(stack) = ctx.data(|d| d.get_temp::<Arc<FontStack>>(egui::Id::new("terminal-fonts")))
    {
        return Ok(stack);
    }
    let styles = StyleFaces::default();
    let stack = if let Some(path) = std::env::var_os("NEKOMATA_FONT") {
        FontStack::discover_in_files(&[path.into()], None, &[], &[], &styles)?
    } else {
        FontStack::auto_discover(None, &[], &[], &styles)?
    };
    let stack = Arc::new(stack);
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("terminal-fonts"), stack.clone()));
    Ok(stack)
}

struct CachedRow {
    frame: RowFrame,
    backgrounds: Arc<Mesh>,
    glyphs: Vec<Arc<Mesh>>,
    decorations: Arc<Mesh>,
}

pub struct Renderer {
    font_size: f32,
    ppp: f32,
    glyphs: GlyphIndex,
    walker: GridWalker,
    shaper: Shaper,
    shaped: ShapeFrame,
    mono: TextureHandle,
    color: TextureHandle,
    theme: ResolvedTheme,
    rows: Vec<CachedRow>,
    geometry: Option<(Rect, u16, u16)>,
    scratch: RowPaint,
    colors: HashMap<[u32; 4], Color32>,
    /// Useful both for the release benchmark and damage regression tests.
    pub rows_painted: usize,
}

impl Renderer {
    pub fn new(ctx: &egui::Context, font_size: f32, ppp: f32) -> anyhow::Result<Self> {
        let stack = load_fonts(ctx)?;
        let side = NonZeroU32::new(GLYPH_ATLAS_SIDE).unwrap();
        let mono = ctx.load_texture(
            "terminal mono",
            ColorImage::filled([side.get() as usize; 2], Color32::TRANSPARENT),
            TextureOptions::NEAREST,
        );
        let color = ctx.load_texture(
            "terminal color",
            ColorImage::filled(
                [color_atlas_side(side).get() as usize; 2],
                Color32::TRANSPARENT,
            ),
            TextureOptions::NEAREST,
        );
        let theme =
            Theme::with_overrides(Some("#d8dee9"), Some("#14171f")).with_cursor(Some("#a3be8c"));
        Ok(Self {
            font_size,
            ppp,
            glyphs: GlyphIndex::new((*stack).clone(), font_size * ppp, side),
            walker: GridWalker::default(),
            shaper: Shaper::new(),
            shaped: ShapeFrame::empty(),
            mono,
            color,
            theme: ResolvedTheme::new(&theme),
            rows: Vec::new(),
            geometry: None,
            scratch: RowPaint::default(),
            colors: HashMap::new(),
            rows_painted: 0,
        })
    }

    pub fn matches(&self, font_size: f32, ppp: f32) -> bool {
        self.font_size == font_size && self.ppp == ppp
    }

    pub fn cell_size(&self) -> Vec2 {
        let m = self.glyphs.cell_metrics();
        Vec2::new(m.width as f32, m.height as f32) / self.ppp
    }

    pub fn paint(
        &mut self,
        painter: &egui::Painter,
        shadow: &ShadowScreen,
        rect: Rect,
        focused: bool,
        selection: Option<(usize, usize)>,
    ) {
        let screen = shadow.screen();
        let resets = self.glyphs.atlas_resets();
        self.walker.walk(
            &mut self.glyphs,
            screen,
            &mut self.shaper,
            &[],
            &mut self.shaped,
        );
        // A recycle invalidates UVs allocated earlier in the walk. Retry once,
        // as felis does; request another frame if this screen exceeds the atlas.
        if self.glyphs.atlas_was_reset() {
            self.walker.walk(
                &mut self.glyphs,
                screen,
                &mut self.shaper,
                &[],
                &mut self.shaped,
            );
        }
        let recycled = self.glyphs.atlas_resets() != resets;
        let retry = self.glyphs.atlas_was_reset();
        for upload in self.glyphs.drain_pending() {
            let a = upload.alloc;
            let size = [a.width as usize, a.height as usize];
            let (texture, pixels) = match upload.pixels {
                GlyphPixels::Coverage(bytes) => (
                    &mut self.mono,
                    bytes.into_iter().map(Color32::from_white_alpha).collect(),
                ),
                GlyphPixels::Rgba(bytes) => (
                    &mut self.color,
                    bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|c| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]))
                        .collect(),
                ),
            };
            texture.set_partial(
                [a.x as usize, a.y as usize],
                ColorImage::new(size, pixels),
                TextureOptions::NEAREST,
            );
        }
        let cols = usize::from(screen.cols());
        let selection = selection.filter(|_| cols > 0).map(|(a, b)| {
            let first = a.min(b);
            let last = a.max(b);
            SelectionRange {
                start: ((first / cols) as u16, (first % cols) as u16),
                end: ((last / cols) as u16, (last % cols) as u16),
                rectangle: false,
            }
        });
        let frame = FrameState::for_screen(
            screen,
            focused,
            shadow.viewport(),
            selection,
            shadow.reverse_video(),
            None,
            0,
        );
        let geometry = (rect, screen.rows(), screen.cols());
        let all = self.geometry != Some(geometry) || recycled;
        self.geometry = Some(geometry);
        self.rows.truncate(usize::from(screen.rows()));
        self.rows_painted = 0;
        let cells = CellPainter::new(
            screen,
            self.glyphs.cell_metrics(),
            &self.theme,
            &self.glyphs,
            &self.shaped,
        )
        .with_cursor_visible(focused)
        .with_selection(selection)
        .with_reverse_video(shadow.reverse_video())
        .with_viewport(shadow.viewport(), shadow.viewport_max())
        .resolved();
        for row in 0..screen.rows() {
            let at = usize::from(row);
            let Some(row_cells) = RowCells::of_screen(screen, row) else {
                continue;
            };
            let row_frame = frame.row(row, &row_cells);
            if !all
                && !screen.damage().is_dirty(at)
                && self.rows.get(at).is_some_and(|c| c.frame == row_frame)
            {
                continue;
            }
            self.rows_painted += 1;
            self.scratch.clear();
            cells.extend_row(&mut self.scratch, row);
            let cached = encode(
                &self.scratch,
                row_frame,
                Geometry {
                    rect,
                    ppp: self.ppp,
                    grid_width: f32::from(screen.cols()) * self.glyphs.cell_metrics().width as f32,
                },
                &self.mono,
                &self.color,
                &mut self.colors,
            );
            if at < self.rows.len() {
                self.rows[at] = cached;
            } else {
                self.rows.push(cached);
            }
        }
        // Keep the upstream pass order across rows: ink may overhang its cell.
        for row in &self.rows {
            painter.add(Shape::Mesh(row.backgrounds.clone()));
        }
        for row in &self.rows {
            for mesh in &row.glyphs {
                painter.add(Shape::Mesh(mesh.clone()));
            }
        }
        for row in &self.rows {
            painter.add(Shape::Mesh(row.decorations.clone()));
        }
        if retry {
            self.geometry = None;
            painter.ctx().request_repaint();
        }
    }
}

fn srgb(color: [f32; 4], cache: &mut HashMap<[u32; 4], Color32>) -> Color32 {
    // Truecolor output can stream arbitrary colors; keep the memo bounded.
    if cache.len() >= 4096 {
        cache.clear();
    }
    *cache.entry(color.map(f32::to_bits)).or_insert_with(|| {
        let [r, g, b, a] = linear_rgba_to_srgb8(color);
        Color32::from_rgba_unmultiplied(r, g, b, a)
    })
}

fn bounds(origin: Pos2, ppp: f32, pos: [f32; 2], size: [f32; 2]) -> Rect {
    Rect::from_min_size(origin + Vec2::from(pos) / ppp, Vec2::from(size) / ppp)
}

struct Geometry {
    rect: Rect,
    ppp: f32,
    grid_width: f32,
}

fn encode(
    paint: &RowPaint,
    frame: RowFrame,
    geometry: Geometry,
    mono: &TextureHandle,
    color: &TextureHandle,
    colors: &mut HashMap<[u32; 4], Color32>,
) -> CachedRow {
    let Geometry {
        rect,
        ppp,
        grid_width,
    } = geometry;
    let origin = rect.min;
    let mut backgrounds = Mesh::default();
    for r in &paint.rects {
        let mut band = bounds(origin, ppp, r.origin_px, r.size_px);
        if r.origin_px[0] + r.size_px[0] >= grid_width {
            band.max.x = band.max.x.max(rect.right());
        }
        backgrounds.add_colored_rect(band, srgb(r.color, colors));
    }
    let mut glyphs: Vec<Arc<Mesh>> = Vec::new();
    let mut mesh = Mesh::with_texture(mono.id());
    for g in &paint.glyphs {
        let texture = match g.atlas {
            AtlasKind::Mono => mono,
            AtlasKind::Color => color,
        };
        if mesh.texture_id != texture.id() {
            if !mesh.is_empty() {
                glyphs.push(Arc::new(mesh));
            }
            mesh = Mesh::with_texture(texture.id());
        }
        let side = texture.size()[0] as f32;
        let uv = Rect::from_min_max(
            Pos2::from(g.src_min_px) / side,
            Pos2::from(g.src_max_px) / side,
        );
        mesh.add_rect_with_uv(
            bounds(origin, ppp, g.origin_px, g.size_px),
            uv,
            srgb(g.color, colors),
        );
    }
    if !mesh.is_empty() {
        glyphs.push(Arc::new(mesh));
    }
    let mut decorations = Mesh::default();
    for d in &paint.decos {
        let ink = srgb(d.color, colors);
        let rect = bounds(origin, ppp, d.origin_px, d.size_px);
        match d.kind {
            DecoKind::Solid => decorations.add_colored_rect(rect, ink),
            DecoKind::Dotted | DecoKind::Dashed => {
                let period = (d.period_px / ppp).max(1.0 / ppp);
                let width = if d.kind == DecoKind::Dotted {
                    d.thickness_px / ppp
                } else {
                    period * 0.5
                };
                let mut x = rect.left();
                while x < rect.right() {
                    decorations.add_colored_rect(
                        Rect::from_min_max(
                            Pos2::new(x, rect.top()),
                            Pos2::new((x + width).min(rect.right()), rect.bottom()),
                        ),
                        ink,
                    );
                    x += period;
                }
            }
            DecoKind::Curly => {
                let step = 1.0 / ppp;
                let period = (d.period_px / ppp).max(step);
                let thickness = d.thickness_px / ppp;
                let mut x = rect.left();
                while x < rect.right() {
                    let phase = (x - rect.left()) / period * std::f32::consts::TAU;
                    let y =
                        rect.center().y + phase.sin() * (rect.height() - thickness).max(0.0) * 0.5;
                    decorations.add_colored_rect(
                        Rect::from_min_size(
                            Pos2::new(x, y - thickness * 0.5),
                            Vec2::new(step.min(rect.right() - x), thickness),
                        ),
                        ink,
                    );
                    x += step;
                }
            }
        }
    }
    CachedRow {
        frame,
        backgrounds: Arc::new(backgrounds),
        glyphs,
        decorations: Arc::new(decorations),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use felis_grid::{
        Cell, Grapheme, StyleTable,
        wire::{RowEncode, encode_row},
    };
    use felis_protocol::{RowPayload, messages::GridMsg};

    fn write_row(shadow: &mut ShadowScreen, row: u16, text: &str) {
        let mut cells: Vec<Cell> = text
            .chars()
            .map(|c| Cell {
                grapheme: Grapheme::Char(c),
                ..Cell::default()
            })
            .collect();
        cells.resize(usize::from(shadow.screen().cols()), Cell::default());
        let bytes = encode_row(
            RowEncode {
                cells: &cells,
                pad_to: cells.len(),
                sized_cells: &[],
                soft_wrap_continued: false,
            },
            &StyleTable::new(),
        )
        .unwrap();
        shadow
            .apply(&GridMsg::RowDelta {
                rows: vec![(row, RowPayload(bytes))],
            })
            .unwrap();
    }

    fn draw(
        ctx: &egui::Context,
        renderer: &mut Renderer,
        shadow: &mut ShadowScreen,
        rect: Rect,
        focused: bool,
        selection: Option<(usize, usize)>,
    ) -> egui::FullOutput {
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx
                .layer_painter(egui::LayerId::background())
                .with_clip_rect(rect);
            renderer.paint(&painter, shadow, rect, focused, selection);
        });
        shadow.clear_damage();
        output
    }

    #[test]
    fn last_column_background_reaches_the_pane_edge_without_stretching_rows() {
        let ctx = egui::Context::default();
        let mut renderer = Renderer::new(&ctx, 15.0, 1.0).unwrap();
        let cell = renderer.cell_size();
        let rect = Rect::from_min_size(
            Pos2::new(12.0, 20.0),
            Vec2::new(cell.x * 3.0 + 3.5, cell.y * 2.0),
        );
        let mut shadow = ShadowScreen::new(2, 3);
        write_row(&mut shadow, 0, "abc");
        draw(&ctx, &mut renderer, &mut shadow, rect, false, None);
        let mesh = &renderer.rows[0].backgrounds;
        assert_eq!(mesh.calc_bounds().right(), rect.right());
        assert_eq!(mesh.calc_bounds().bottom(), rect.top() + cell.y);
    }
    #[test]
    fn unchanged_frames_reuse_meshes_without_uploading_glyphs() {
        let ctx = egui::Context::default();
        let mut renderer = Renderer::new(&ctx, 15.0, 1.0).unwrap();
        let mut shadow = ShadowScreen::new(3, 8);
        write_row(&mut shadow, 0, "hello");
        let rect = Rect::from_min_size(Pos2::ZERO, renderer.cell_size() * Vec2::new(8.0, 3.0));
        let first = draw(&ctx, &mut renderer, &mut shadow, rect, false, None);
        assert!(!first.textures_delta.set.is_empty());
        let mesh = renderer.rows[0].glyphs[0].clone();
        let next = draw(&ctx, &mut renderer, &mut shadow, rect, false, None);
        assert_eq!(renderer.rows_painted, 0);
        assert!(Arc::ptr_eq(&mesh, &renderer.rows[0].glyphs[0]));
        assert!(next.textures_delta.set.is_empty());
    }

    #[test]
    fn row_delta_rebuilds_only_the_changed_row() {
        let ctx = egui::Context::default();
        let mut renderer = Renderer::new(&ctx, 15.0, 1.0).unwrap();
        let mut shadow = ShadowScreen::new(3, 8);
        write_row(&mut shadow, 0, "hello");
        write_row(&mut shadow, 1, "hello");
        let rect = Rect::from_min_size(Pos2::ZERO, renderer.cell_size() * Vec2::new(8.0, 3.0));
        draw(&ctx, &mut renderer, &mut shadow, rect, false, None);
        let unchanged = renderer.rows[0].glyphs[0].clone();
        let changed = renderer.rows[1].glyphs[0].clone();
        write_row(&mut shadow, 1, "new text");
        let output = draw(&ctx, &mut renderer, &mut shadow, rect, false, None);
        assert_eq!(renderer.rows_painted, 1);
        assert!(Arc::ptr_eq(&unchanged, &renderer.rows[0].glyphs[0]));
        assert!(!Arc::ptr_eq(&changed, &renderer.rows[1].glyphs[0]));
        assert!(!output.textures_delta.set.is_empty());
    }

    #[test]
    fn cursor_focus_and_selection_invalidate_their_previous_rows() {
        let ctx = egui::Context::default();
        let mut renderer = Renderer::new(&ctx, 15.0, 1.0).unwrap();
        let mut shadow = ShadowScreen::new(3, 8);
        let rect = Rect::from_min_size(Pos2::ZERO, renderer.cell_size() * Vec2::new(8.0, 3.0));
        draw(&ctx, &mut renderer, &mut shadow, rect, false, None);
        draw(&ctx, &mut renderer, &mut shadow, rect, true, None);
        assert_eq!(renderer.rows_painted, 1);
        shadow
            .apply(&GridMsg::CursorState {
                row: 2,
                col: 3,
                visible: true,
                style: felis_grid::CursorStyle::Bar,
                blink: false,
            })
            .unwrap();
        draw(&ctx, &mut renderer, &mut shadow, rect, true, None);
        assert_eq!(renderer.rows_painted, 2);
        assert!(renderer.rows[0].frame.cursor.is_none());
        assert!(renderer.rows[2].frame.cursor.is_some());
        draw(&ctx, &mut renderer, &mut shadow, rect, true, Some((8, 11)));
        assert_eq!(renderer.rows_painted, 1);
        assert_eq!(renderer.rows[1].frame.selection, Some((0, 3)));
        draw(&ctx, &mut renderer, &mut shadow, rect, false, None);
        assert_eq!(renderer.rows_painted, 2);
        assert!(renderer.rows[1].frame.selection.is_none());
        assert!(renderer.rows[2].frame.cursor.is_none());
    }

    #[test]
    fn moving_a_pane_repositions_cached_meshes() {
        let ctx = egui::Context::default();
        let mut renderer = Renderer::new(&ctx, 15.0, 1.0).unwrap();
        let mut shadow = ShadowScreen::new(2, 4);
        let rect = Rect::from_min_size(Pos2::ZERO, renderer.cell_size() * Vec2::new(4.0, 2.0));
        draw(&ctx, &mut renderer, &mut shadow, rect, false, None);
        let moved = rect.translate(Vec2::new(20.0, 40.0));
        draw(&ctx, &mut renderer, &mut shadow, moved, false, None);
        assert_eq!(renderer.rows_painted, 2);
        assert_eq!(renderer.rows[0].backgrounds.calc_bounds().min, moved.min);
    }
}
