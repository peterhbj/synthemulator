use eframe::egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Style, TextStyle,
    Visuals,
};

pub const BG: Color32 = Color32::from_rgb(0x09, 0x09, 0x0b);
pub const SURFACE: Color32 = Color32::from_rgb(0x11, 0x11, 0x13);
pub const ELEVATED: Color32 = Color32::from_rgb(0x19, 0x19, 0x1d);
pub const FG: Color32 = Color32::from_rgb(0xf3, 0xf3, 0xf1);
pub const MUTED: Color32 = Color32::from_rgb(0x9c, 0x9c, 0xa4);
pub const SUBTLE: Color32 = Color32::from_rgb(0x6d, 0x6d, 0x75);
pub const ACCENT: Color32 = Color32::from_rgb(0xc8, 0xcf, 0xd8);
pub const ACCENT_FG: Color32 = Color32::from_rgb(0x09, 0x09, 0x0b);
pub const BORDER: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x30);
pub const KEY: Color32 = Color32::from_rgb(0xec, 0xea, 0xe3);
pub const KEY_FG: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1c);
pub const KEY_SHARP: Color32 = Color32::from_rgb(0x16, 0x16, 0x18);
pub const KEY_SHARP_FG: Color32 = Color32::from_rgb(0xc8, 0xc8, 0xce);
pub const GLOW: Color32 = Color32::from_rgb(0xd7, 0xde, 0xe6);

pub fn panel_stroke() -> Stroke {
    Stroke::new(1.0, Color32::from_white_alpha(18))
}

pub fn rounding_xl() -> CornerRadius {
    CornerRadius::same(24)
}

pub fn rounding_lg() -> CornerRadius {
    CornerRadius::same(16)
}

pub fn rounding_md() -> CornerRadius {
    CornerRadius::same(12)
}

pub fn rounding_sm() -> CornerRadius {
    CornerRadius::same(8)
}

pub fn install_fonts(ctx: &eframe::egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "plex".into(),
        FontData::from_static(include_bytes!("../../assets/fonts/IBMPlexSans-Regular.ttf")).into(),
    );
    fonts.font_data.insert(
        "plex-medium".into(),
        FontData::from_static(include_bytes!("../../assets/fonts/IBMPlexSans-Medium.ttf")).into(),
    );
    fonts.font_data.insert(
        "plex-semibold".into(),
        FontData::from_static(include_bytes!(
            "../../assets/fonts/IBMPlexSans-SemiBold.ttf"
        ))
        .into(),
    );
    fonts.font_data.insert(
        "plex-mono".into(),
        FontData::from_static(include_bytes!("../../assets/fonts/IBMPlexMono-Regular.ttf")).into(),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "plex".to_owned());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "plex-mono".to_owned());
    fonts.families.insert(
        FontFamily::Name("plex-medium".into()),
        vec!["plex-medium".into()],
    );
    fonts.families.insert(
        FontFamily::Name("plex-semibold".into()),
        vec!["plex-semibold".into()],
    );
    ctx.set_fonts(fonts);
}

pub fn install_style(ctx: &eframe::egui::Context) {
    let mut style = Style {
        visuals: Visuals::dark(),
        ..Style::default()
    };
    style.visuals.dark_mode = true;
    style.visuals.override_text_color = Some(FG);
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = SURFACE;
    style.visuals.extreme_bg_color = ELEVATED;
    style.visuals.faint_bg_color = SURFACE;
    style.visuals.widgets.noninteractive.bg_fill = SURFACE;
    style.visuals.widgets.inactive.bg_fill = ELEVATED;
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, MUTED);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x22, 0x22, 0x28);
    style.visuals.widgets.active.bg_fill = ACCENT;
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, ACCENT_FG);
    style.visuals.selection.bg_fill = ACCENT;
    style.visuals.selection.stroke = Stroke::new(1.0, ACCENT_FG);
    style.visuals.widgets.inactive.corner_radius = rounding_sm();
    style.visuals.widgets.hovered.corner_radius = rounding_sm();
    style.visuals.widgets.active.corner_radius = rounding_sm();
    style.spacing.item_spacing = eframe::egui::vec2(8.0, 8.0);
    style.spacing.button_padding = eframe::egui::vec2(10.0, 6.0);

    let medium = FontFamily::Name("plex-medium".into());
    let semibold = FontFamily::Name("plex-semibold".into());
    style
        .text_styles
        .insert(TextStyle::Small, FontId::new(11.0, medium.clone()));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::new(12.0, medium));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::new(22.0, semibold));
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(11.0, FontFamily::Monospace),
    );
    ctx.set_style(style);
}

pub fn section_label(text: &str) -> eframe::egui::RichText {
    eframe::egui::RichText::new(text)
        .size(10.5)
        .color(SUBTLE)
        .extra_letter_spacing(1.6)
}
