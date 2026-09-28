//! The artistic controls (task 12), generated from
//! [`settings::CONTROLS`](pigment_core::settings::CONTROLS): one slider per
//! entry, with its label, bounded range, displayed value, the meaning of
//! each end, and a reset to its default.
//!
//! Keyboard, when a slider has focus: Left/Right (or Down/Up) step by 0.01,
//! Page Down/Page Up by 0.1, Home/End jump to the ends, and Delete or
//! Backspace resets to the default. The Reset action beside the figure
//! appears (and takes focus) only while the value differs from the default;
//! a notch under the track marks where the default is.

use eframe::egui;
use pigment_core::palette::PALETTES;
use pigment_core::settings::{CONTROLS, Channel, ControlSpec, Group, PaletteId};

use crate::theme;

/// Keyboard and slider step.
pub const STEP: f64 = 0.01;
/// Page Up / Page Down step.
pub const PAGE_STEP: f64 = 0.1;

/// What happened to one control this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Edit {
    /// The value changed.
    pub changed: bool,
    /// The change came from a continuing gesture (drag or held key), so an
    /// interaction preview is wanted; otherwise render settled at once.
    pub interacting: bool,
}

impl Edit {
    pub fn merge(self, other: Edit) -> Edit {
        Edit {
            changed: self.changed || other.changed,
            interacting: self.interacting || other.interacting,
        }
    }
}

/// Rounds to the slider step, so keyboard and drag values stay tidy.
fn snap(spec: &ControlSpec, v: f64) -> f64 {
    ((v / STEP).round() * STEP).clamp(spec.min, spec.max)
}

pub fn format_value(v: f64) -> String {
    format!("{v:.2}")
}

/// Text shown to people and assistive technology for a value, including
/// which described end it is nearer.
pub fn value_text(spec: &ControlSpec, v: f64) -> String {
    let t = (v - spec.min) / (spec.max - spec.min);
    let lean = if (v - spec.default).abs() < STEP / 2.0 {
        "default".to_string()
    } else if t < 0.5 {
        format!("nearer {}", spec.low)
    } else {
        format!("nearer {}", spec.high)
    };
    format!("{} ({lean})", format_value(v))
}

/// Which channel heading a control sits under.
pub fn channel_heading(c: Channel) -> (&'static str, &'static str) {
    match c {
        Channel::Structure => ("Structure", "Moves the landscape: recomposes the painting."),
        Channel::PaintHandling => (
            "Paint handling",
            "How the paint is laid down. The landscape stays put.",
        ),
        Channel::Appearance => ("Appearance", "Color and air. The landscape stays put."),
    }
}

/// Controls in `group`, grouped by channel in display order.
pub fn grouped(group: Group) -> Vec<(Channel, Vec<&'static ControlSpec>)> {
    let mut out: Vec<(Channel, Vec<&'static ControlSpec>)> = Vec::new();
    for ch in [
        Channel::Structure,
        Channel::PaintHandling,
        Channel::Appearance,
    ] {
        let specs: Vec<_> = CONTROLS
            .iter()
            .filter(|c| c.group == group && c.channel == ch)
            .collect();
        if !specs.is_empty() || (group == Group::Main && ch == Channel::Appearance) {
            out.push((ch, specs));
        }
    }
    out
}

/// One slider row: label and figure, the track with a notch at the
/// default, and the meaning of the two ends.
pub fn slider(ui: &mut egui::Ui, spec: &ControlSpec, value: &mut f64) -> Edit {
    // The row's tight spacing stays inside the row.
    ui.scope(|ui| slider_row(ui, spec, value)).inner
}

fn slider_row(ui: &mut egui::Ui, spec: &ControlSpec, value: &mut f64) -> Edit {
    ui.spacing_mut().item_spacing.y = 0.0;
    let before = *value;
    let mut edit = Edit::default();
    let off_default = (*value - spec.default).abs() >= STEP / 2.0;
    let label_resp = ui
        .horizontal(|ui| {
            let label = ui.label(egui::RichText::new(spec.label).color(theme::INK));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(format_value(*value)).monospace().color(
                    if off_default {
                        theme::INK
                    } else {
                        theme::INK_2
                    },
                ));
                if off_default {
                    let reset = ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Reset").small().color(theme::INK),
                            )
                            .small(),
                        )
                        .on_hover_text(format!(
                            "Back to the default, {} (Delete)",
                            format_value(spec.default)
                        ));
                    reset.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            true,
                            format!("Reset {}", spec.label),
                        )
                    });
                    if reset.clicked() {
                        *value = spec.default;
                    }
                }
            });
            label
        })
        .inner;

    ui.spacing_mut().slider_width = ui.available_width();
    let resp = ui
        .add(
            egui::Slider::new(value, spec.min..=spec.max)
                .show_value(false)
                .step_by(STEP)
                .clamping(egui::SliderClamping::Always),
        )
        .labelled_by(label_resp.id);
    // egui draws its handle in the rail's colour; draw a light one on top
    // (accent while dragged or focused), and a notch where the default is.
    let r = resp.rect;
    let inset = r.height() / 2.5;
    let at = |v: f64| {
        let t = ((v - spec.min) / (spec.max - spec.min)) as f32;
        egui::lerp((r.left() + inset)..=(r.right() - inset), t)
    };
    let x = at(spec.default);
    let notch = egui::Rect::from_min_max(
        egui::pos2(x - 0.75, r.center().y + 4.0),
        egui::pos2(x + 0.75, r.center().y + 9.0),
    );
    ui.painter().rect_filled(notch, 0.0, theme::INK_3);
    let engaged = resp.has_focus() || resp.dragged();
    let handle = if engaged {
        theme::ACCENT
    } else if resp.hovered() {
        egui::Color32::WHITE
    } else {
        theme::INK
    };
    ui.painter().circle(
        egui::pos2(at(*value), r.center().y),
        inset * 0.8,
        handle,
        egui::Stroke::new(2.0, theme::PANEL),
    );
    if resp.has_focus() {
        ui.painter().rect_stroke(
            r.expand2(egui::vec2(4.0, 1.0)),
            egui::CornerRadius::same(theme::RADIUS),
            egui::Stroke::new(1.5, theme::ACCENT),
            egui::StrokeKind::Outside,
        );
        let keys = ui.input_mut(|i| {
            [
                egui::Key::PageUp,
                egui::Key::PageDown,
                egui::Key::Home,
                egui::Key::End,
                egui::Key::Delete,
                egui::Key::Backspace,
            ]
            .map(|k| i.consume_key(egui::Modifiers::NONE, k))
        });
        let [up, down, home, end, del, back] = keys;
        if up {
            *value = snap(spec, *value + PAGE_STEP);
        }
        if down {
            *value = snap(spec, *value - PAGE_STEP);
        }
        if home {
            *value = spec.min;
        }
        if end {
            *value = spec.max;
        }
        if del || back {
            *value = spec.default;
        }
    }
    let text = value_text(spec, *value);
    resp.widget_info(|| {
        let mut info = egui::WidgetInfo::slider(true, *value, spec.label);
        info.current_text_value = Some(text.clone());
        info
    });
    resp.clone().on_hover_text(format!(
        "{}\n{} → {}\nIn the recipe: {}",
        value_text(spec, *value),
        spec.low,
        spec.high,
        spec.key
    ));
    ui.add_space(1.0);
    // The two ends, each in its own half so long words wrap, not collide.
    ui.columns(2, |c| {
        let small = |t: &str| egui::Label::new(egui::RichText::new(t).small().color(theme::INK_3));
        c[0].with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.add(small(spec.low).wrap());
        });
        c[1].with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
            ui.add(small(spec.high).wrap().halign(egui::Align::Max));
        });
    });

    if *value != before {
        edit.changed = true;
        // A drag, or arrow keys that may repeat: render small now and
        // settled after the input stops. Clicks and jumps render settled.
        edit.interacting = resp.dragged() || (resp.changed() && resp.has_focus());
    }
    edit
}

/// The palette picker.
pub fn palette_picker(ui: &mut egui::Ui, id: &mut PaletteId) -> Edit {
    let before = *id;
    let label = ui.label(egui::RichText::new("Palette").color(theme::INK));
    let name = |id: PaletteId| PALETTES.iter().find(|p| p.id == id).map_or("?", |p| p.name);
    egui::ComboBox::from_id_salt("palette")
        .selected_text(name(*id))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for p in PALETTES {
                ui.selectable_value(id, p.id, p.name);
            }
        })
        .response
        .labelled_by(label.id);
    Edit {
        changed: *id != before,
        interacting: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_control_is_shown_once_under_its_channel() {
        let mut seen = Vec::new();
        for group in [Group::Main, Group::Advanced] {
            for (ch, specs) in grouped(group) {
                for s in specs {
                    assert_eq!((s.channel, s.group), (ch, group), "{}", s.key);
                    seen.push(s.key);
                }
            }
        }
        seen.sort();
        let mut all: Vec<_> = CONTROLS.iter().map(|c| c.key).collect();
        all.sort();
        assert_eq!(seen, all);
        // Form (structure) is apart from the paint handling pair.
        let main = grouped(Group::Main);
        assert_eq!(main[0].0, Channel::Structure);
        assert_eq!(main[0].1[0].key, "form.faceting");
    }

    #[test]
    fn value_text_names_the_nearer_end() {
        let f = CONTROLS[0];
        assert_eq!(value_text(&f, f.default), "0.55 (default)");
        assert!(value_text(&f, 0.1).ends_with(&format!("(nearer {})", f.low)));
        assert!(value_text(&f, 0.9).ends_with(&format!("(nearer {})", f.high)));
        let m = CONTROLS
            .iter()
            .find(|c| c.key == "painting.mark_scale")
            .unwrap();
        assert_eq!(snap(m, 2.3), 2.0);
        assert_eq!(snap(m, 0.43), 0.5);
        assert!((snap(m, 1.234) - 1.23).abs() < 1e-9);
    }
}
