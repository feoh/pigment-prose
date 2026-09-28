//! The studio's visual system: a graphite working console whose only
//! colour is the painting (direction contract in
//! `.impeccable/surfaces/crates-pigment-studio-src-app-rs.md`, documented
//! in `DESIGN.md`).
//!
//! - Near-neutral graphite so the painting's colours read truly; panels
//!   are divided by 1 px seams, never by shadows.
//! - One cool accent for the primary action, keyboard focus and slider
//!   fill; amber for pending and unsaved; coral for errors. Nothing else is
//!   coloured.
//! - Atkinson Hyperlegible Next for text (chosen for legibility in long
//!   sessions), Atkinson Hyperlegible Mono for figures. Both are bundled
//!   under the SIL Open Font License 1.1 (`fonts/OFL*.txt`).

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow,
    Stroke, TextStyle, Visuals,
};

/// Where the painting sits.
pub const SURROUND: Color32 = Color32::from_rgb(0x11, 0x12, 0x14);
/// Panels: top bar, control column, status line.
pub const PANEL: Color32 = Color32::from_rgb(0x1a, 0x1c, 0x20);
/// One step up: buttons, menus, the modal, the state chip.
pub const RAISED: Color32 = Color32::from_rgb(0x22, 0x25, 0x2a);
/// Text-entry wells.
pub const FIELD: Color32 = Color32::from_rgb(0x15, 0x17, 0x1a);
pub const HOVER: Color32 = Color32::from_rgb(0x2a, 0x2e, 0x34);
/// 1 pt dividers between panels.
pub const SEAM: Color32 = Color32::from_rgb(0x2d, 0x30, 0x36);
pub const SEAM_STRONG: Color32 = Color32::from_rgb(0x3d, 0x42, 0x4a);

pub const INK: Color32 = Color32::from_rgb(0xe4, 0xe6, 0xea);
/// Secondary text (≥ 7:1 on the panel).
pub const INK_2: Color32 = Color32::from_rgb(0xa9, 0xae, 0xb7);
/// Tertiary text: endpoint words, units (≥ 4.5:1 on the panel).
pub const INK_3: Color32 = Color32::from_rgb(0x8f, 0x95, 0x9f);

/// The one accent: primary action, focus, slider fill.
pub const ACCENT: Color32 = Color32::from_rgb(0x6c, 0xc0, 0xe5);
/// Text on an accent fill.
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x0b, 0x1a, 0x22);
/// The accent muted for areas under text (selection, slider fill).
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x2f, 0x61, 0x79);
/// Pending work and unsaved changes.
pub const AMBER: Color32 = Color32::from_rgb(0xe0, 0xa6, 0x4a);
pub const CORAL: Color32 = Color32::from_rgb(0xef, 0x7a, 0x6d);

pub const RADIUS: u8 = 3;

/// Font family for SemiBold text (headers, the wordmark).
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

fn fonts() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    let add = |f: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        f.font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    };
    add(
        &mut f,
        "atkinson",
        include_bytes!("../fonts/AtkinsonHyperlegibleNext-Regular.ttf"),
    );
    add(
        &mut f,
        "atkinson-semibold",
        include_bytes!("../fonts/AtkinsonHyperlegibleNext-SemiBold.ttf"),
    );
    add(
        &mut f,
        "atkinson-mono",
        include_bytes!("../fonts/AtkinsonHyperlegibleMono-Regular.ttf"),
    );
    // Ours first; egui's bundled fonts stay as fallbacks for scripts and
    // symbols Atkinson does not cover (so any prose still renders).
    let prop = f.families.entry(FontFamily::Proportional).or_default();
    prop.insert(0, "atkinson".into());
    let mono = f.families.entry(FontFamily::Monospace).or_default();
    mono.insert(0, "atkinson-mono".into());
    let fallbacks = f.families[&FontFamily::Proportional].clone();
    let mut bold = vec!["atkinson-semibold".to_string()];
    bold.extend(fallbacks.into_iter().filter(|n| n != "atkinson"));
    f.families.insert(semibold(), bold);
    f
}

fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    let r = CornerRadius::same(RADIUS);
    v.override_text_color = None;
    v.weak_text_color = Some(INK_3);
    v.panel_fill = PANEL;
    v.window_fill = RAISED;
    v.extreme_bg_color = SURROUND;
    v.text_edit_bg_color = Some(FIELD);
    v.faint_bg_color = Color32::from_rgb(0x1f, 0x22, 0x26);
    v.code_bg_color = RAISED;
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = AMBER;
    v.error_fg_color = CORAL;
    v.window_stroke = Stroke::new(1.0, SEAM_STRONG);
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(4);
    v.window_shadow = Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(140),
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 14,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, INK);
    v.text_cursor.stroke = Stroke::new(2.0, ACCENT);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.striped = true;
    v.indent_has_left_vline = false;
    v.collapsing_header_frame = false;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, SEAM);
    w.noninteractive.fg_stroke = Stroke::new(1.0, INK);
    w.noninteractive.corner_radius = r;

    // Resting controls: a quiet raised fill, a seam, primary ink.
    w.inactive.bg_fill = SEAM_STRONG; // slider rail, checkbox well
    w.inactive.weak_bg_fill = RAISED; // buttons, combo boxes
    w.inactive.bg_stroke = Stroke::new(1.0, SEAM_STRONG);
    w.inactive.fg_stroke = Stroke::new(1.0, INK);
    w.inactive.corner_radius = r;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = Color32::from_rgb(0xd8, 0xdc, 0xe2);
    w.hovered.weak_bg_fill = HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x55, 0x5b, 0x65));
    w.hovered.fg_stroke = Stroke::new(1.0, INK);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;

    // Pressed or keyboard-focused: the accent ring. Focus must be
    // unmistakable (keyboard-first).
    w.active.bg_fill = ACCENT;
    w.active.weak_bg_fill = HOVER;
    w.active.bg_stroke = Stroke::new(2.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.5, INK);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;

    w.open.bg_fill = RAISED;
    w.open.weak_bg_fill = HOVER;
    w.open.bg_stroke = Stroke::new(1.0, SEAM_STRONG);
    w.open.fg_stroke = Stroke::new(1.0, INK);
    w.open.corner_radius = r;
    v
}

/// Installs fonts, colours, type scale and spacing.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        s.visuals = visuals();
        s.text_styles = [
            (TextStyle::Heading, FontId::new(15.0, semibold())),
            (TextStyle::Body, FontId::new(15.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(15.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Small,
                FontId::new(12.5, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(14.0, FontFamily::Monospace),
            ),
        ]
        .into();
        let sp = &mut s.spacing;
        sp.item_spacing = egui::vec2(8.0, 6.0);
        sp.button_padding = egui::vec2(10.0, 5.0);
        sp.interact_size = egui::vec2(28.0, 28.0);
        sp.slider_rail_height = 4.0;
        sp.icon_width = 16.0;
        sp.icon_width_inner = 9.0;
        sp.window_margin = Margin::same(16);
        sp.menu_margin = Margin::same(6);
        s.interaction.tooltip_delay = 0.35;
    });
}

/// A panel header: SemiBold small capitals in secondary ink.
pub fn header(text: &str) -> egui::RichText {
    egui::RichText::new(text.to_uppercase())
        .family(semibold())
        .size(12.5)
        .extra_letter_spacing(0.9)
        .color(INK_2)
}

/// The panel frame: fill and inner margin, no stroke (seams are drawn
/// by the panel separators).
pub fn panel_frame(margin: Margin) -> egui::Frame {
    egui::Frame::new().fill(PANEL).inner_margin(margin)
}

/// A primary button: accent fill, dark ink.
pub fn primary(text: &str) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(text.to_owned())
            .color(ON_ACCENT)
            .family(semibold()),
    )
    .fill(ACCENT)
}

/// Keyboard focus on an accent-filled control: the accent ring would
/// vanish against the fill, so ring it in primary ink instead.
pub fn focus_on_accent(ui: &egui::Ui, resp: &egui::Response) {
    if resp.has_focus() {
        ui.painter().rect_stroke(
            resp.rect.expand(2.0),
            CornerRadius::same(RADIUS + 1),
            Stroke::new(2.0, INK),
            egui::StrokeKind::Outside,
        );
    }
}

/// A quiet text action for the top bar.
pub fn quiet(text: &str) -> egui::Button<'static> {
    egui::Button::new(text.to_owned()).frame_when_inactive(false)
}

/// A small filled dot, for state words.
pub fn dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance of an sRGB colour.
    fn luminance(c: Color32) -> f64 {
        let lin = |u: u8| {
            let s = u as f64 / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_meets_contrast_on_every_ground() {
        for ground in [PANEL, RAISED, SURROUND, HOVER, FIELD] {
            assert!(contrast(INK, ground) >= 7.0, "ink on {ground:?}");
            assert!(contrast(INK_2, ground) >= 4.5, "ink 2 on {ground:?}");
            assert!(contrast(INK_3, ground) >= 4.5, "ink 3 on {ground:?}");
            for c in [ACCENT, AMBER, CORAL] {
                assert!(contrast(c, ground) >= 4.5, "{c:?} on {ground:?}");
            }
        }
        assert!(contrast(ON_ACCENT, ACCENT) >= 7.0);
        assert!(contrast(INK, ACCENT_DIM) >= 4.5, "selected text");
        // Focus ring against the panel (non-text, ≥ 3:1).
        assert!(contrast(ACCENT, PANEL) >= 3.0);
    }

    #[test]
    fn the_greys_are_neutral_enough_to_judge_colour() {
        for c in [SURROUND, PANEL, RAISED, FIELD] {
            let (r, g, b) = (c.r() as i32, c.g() as i32, c.b() as i32);
            let spread = r.max(g).max(b) - r.min(g).min(b);
            assert!(spread <= 8, "{c:?} is tinted");
        }
    }
}
