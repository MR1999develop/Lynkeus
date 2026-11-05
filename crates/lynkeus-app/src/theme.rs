// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Aspetto: token di colore, spaziature, corpi del testo e raggi, applicati una volta al
//! contesto. La vista usa solo questi valori, mai colori o misure scritti a mano.
//!
//! Un solo accento (blu) e solo sugli elementi interattivi; mai rosso né arancio per
//! l'accento. Le corrispondenze evidenziate hanno un fondo proprio, non d'accento, e sono
//! anche in grassetto e sottolineate; le righe selezionate hanno anche un segno sul bordo
//! sinistro: lo stato non è affidato al solo colore. Il focus della tastiera è un bordo di
//! 2 px. I contrasti (testo ≥ 4,5:1, bordi, segni e focus ≥ 3:1) sono verificati da un test.
//!
//! Con un tema a contrasto elevato del sistema ([`SystemContrast`], letto dalla
//! piattaforma all'avvio) i token vengono dai colori di sistema, per il tema chiaro e per
//! lo scuro.

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Theme, ThemePreference, Visuals,
};
use serde::{Deserialize, Serialize};

/// Scala delle spaziature, in punti.
pub const SPACE: [f32; 8] = [4.0, 8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0];
/// Scala dei corpi del testo, in punti: didascalia, piccolo, corpo, sottotitolo, titolo.
pub const FONT: [f32; 5] = [12.0, 13.0, 14.0, 16.0, 20.0];
/// Raggi: controlli, finestre interne e menu.
pub const RADIUS: [u8; 2] = [4, 8];
/// Altezza minima dei controlli interattivi.
pub const INTERACT_HEIGHT: f32 = 24.0;

/// Corpo del testo normale.
pub const BODY_SIZE: f32 = FONT[2];
/// Corpo delle didascalie e dei messaggi sotto i campi.
pub const CAPTION_SIZE: f32 = FONT[0];

/// Larghezza del bordo di focus della tastiera, in punti.
pub const FOCUS_WIDTH: f32 = 2.0;
/// Distanza del bordo di focus da un controllo con il fondo d'accento (il primario), in
/// punti: l'anello sta sul fondo del pannello, non sull'accento.
pub const FOCUS_GAP: f32 = 2.0;
/// Larghezza del segno sul bordo sinistro delle righe selezionate, in punti.
pub const SELECTION_MARK_WIDTH: f32 = 4.0;

/// Nome della famiglia SemiBold, registrata da [`crate::platform::install_fonts`].
pub const SEMIBOLD: &str = "semibold";

/// La famiglia SemiBold.
pub fn semibold_family() -> FontFamily {
    FontFamily::Name(SEMIBOLD.into())
}

/// Tema scelto dall'utente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ThemeChoice {
    /// Segue il sistema (predefinito).
    #[default]
    System,
    /// Sempre chiaro.
    Light,
    /// Sempre scuro.
    Dark,
}

impl ThemeChoice {
    /// Tutte le scelte, nell'ordine del menu.
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];

    /// La preferenza equivalente di egui.
    pub fn preference(self) -> ThemePreference {
        match self {
            ThemeChoice::System => ThemePreference::System,
            ThemeChoice::Light => ThemePreference::Light,
            ThemeChoice::Dark => ThemePreference::Dark,
        }
    }
}

/// Colori dell'interfaccia per un tema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tokens {
    /// Elementi interattivi: bottone primario, link, focus. Solo lì.
    pub accent: Color32,
    /// Testo sopra `accent`.
    pub on_accent: Color32,
    /// Testo principale.
    pub text_strong: Color32,
    /// Testo secondario (didascalie, suggerimenti).
    pub text_weak: Color32,
    /// Bordi di campi e controlli.
    pub stroke_strong: Color32,
    /// Divisori decorativi.
    pub stroke_weak: Color32,
    /// Fondo dei controlli e righe alterne.
    pub fill: Color32,
    /// Fondo dei controlli sotto il puntatore.
    pub fill_hover: Color32,
    /// Fondo dei pannelli.
    pub bg: Color32,
    /// Fondo dei campi di testo e della tabella.
    pub bg_raised: Color32,
    /// Fondo di menu, popup e finestre interne.
    pub bg_overlay: Color32,
    /// Testo e bordi d'errore.
    pub error: Color32,
    /// Testo e bordi d'avviso.
    pub warning: Color32,
    /// Fondo delle corrispondenze evidenziate (non interattive: non è l'accento).
    pub highlight_bg: Color32,
    /// Fondo delle righe e del testo selezionati.
    pub selection_bg: Color32,
    /// Testo sopra `selection_bg`.
    pub on_selection: Color32,
    /// Segno sul bordo sinistro delle righe selezionate.
    pub selection_mark: Color32,
    /// Bordo di focus della tastiera.
    pub focus: Color32,
}

/// I colori di un tema a contrasto elevato del sistema (su Windows `GetSysColor`), da cui
/// derivano tutti i token quando il tema è attivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemContrast {
    /// Fondo delle finestre (`COLOR_WINDOW`).
    pub window: Color32,
    /// Testo delle finestre (`COLOR_WINDOWTEXT`).
    pub window_text: Color32,
    /// Fondo degli elementi selezionati (`COLOR_HIGHLIGHT`).
    pub highlight: Color32,
    /// Testo degli elementi selezionati (`COLOR_HIGHLIGHTTEXT`).
    pub highlight_text: Color32,
    /// Fondo dei bottoni (`COLOR_BTNFACE`).
    pub button_face: Color32,
}

impl Tokens {
    /// Tema chiaro.
    pub const fn light() -> Tokens {
        Tokens {
            accent: Color32::from_rgb(0x00, 0x5F, 0xB8),
            on_accent: Color32::WHITE,
            text_strong: Color32::from_rgb(0x1A, 0x1A, 0x1A),
            text_weak: Color32::from_rgb(0x5C, 0x5C, 0x5C),
            stroke_strong: Color32::from_rgb(0x85, 0x85, 0x85),
            stroke_weak: Color32::from_rgb(0xE0, 0xE0, 0xE0),
            fill: Color32::from_rgb(0xEE, 0xEE, 0xEE),
            fill_hover: Color32::from_rgb(0xE2, 0xE2, 0xE2),
            bg: Color32::from_rgb(0xF7, 0xF7, 0xF7),
            bg_raised: Color32::WHITE,
            bg_overlay: Color32::WHITE,
            error: Color32::from_rgb(0xC4, 0x2B, 0x1C),
            warning: Color32::from_rgb(0x8A, 0x55, 0x00),
            highlight_bg: Color32::from_rgb(0xFF, 0xEB, 0x8A),
            selection_bg: Color32::from_rgb(0xCC, 0xE4, 0xF7),
            on_selection: Color32::from_rgb(0x1A, 0x1A, 0x1A),
            selection_mark: Color32::from_rgb(0x00, 0x5F, 0xB8),
            focus: Color32::from_rgb(0x1A, 0x1A, 0x1A),
        }
    }

    /// Tema scuro.
    pub const fn dark() -> Tokens {
        Tokens {
            accent: Color32::from_rgb(0x60, 0xCD, 0xFF),
            on_accent: Color32::BLACK,
            text_strong: Color32::from_rgb(0xF2, 0xF2, 0xF2),
            text_weak: Color32::from_rgb(0xB3, 0xB3, 0xB3),
            stroke_strong: Color32::from_rgb(0x8A, 0x8A, 0x8A),
            stroke_weak: Color32::from_rgb(0x3A, 0x3A, 0x3A),
            fill: Color32::from_rgb(0x38, 0x38, 0x38),
            fill_hover: Color32::from_rgb(0x45, 0x45, 0x45),
            bg: Color32::from_rgb(0x20, 0x20, 0x20),
            bg_raised: Color32::from_rgb(0x2B, 0x2B, 0x2B),
            bg_overlay: Color32::from_rgb(0x2F, 0x2F, 0x2F),
            error: Color32::from_rgb(0xFF, 0x99, 0xA4),
            warning: Color32::from_rgb(0xFC, 0xE1, 0x00),
            highlight_bg: Color32::from_rgb(0x6B, 0x55, 0x00),
            selection_bg: Color32::from_rgb(0x1F, 0x3F, 0x5C),
            on_selection: Color32::from_rgb(0xF2, 0xF2, 0xF2),
            selection_mark: Color32::from_rgb(0x60, 0xCD, 0xFF),
            focus: Color32::from_rgb(0xF2, 0xF2, 0xF2),
        }
    }

    /// I token di un tema a contrasto elevato: solo i colori di sistema, in coppia (testo
    /// della finestra sul fondo della finestra, testo della selezione sulla selezione).
    /// Le corrispondenze restano riconoscibili da SemiBold e sottolineatura, senza fondo.
    pub const fn high_contrast(colors: &SystemContrast) -> Tokens {
        Tokens {
            accent: colors.highlight,
            on_accent: colors.highlight_text,
            text_strong: colors.window_text,
            text_weak: colors.window_text,
            stroke_strong: colors.window_text,
            stroke_weak: colors.window_text,
            fill: colors.button_face,
            fill_hover: colors.button_face,
            bg: colors.window,
            bg_raised: colors.window,
            bg_overlay: colors.window,
            error: colors.window_text,
            warning: colors.window_text,
            highlight_bg: colors.window,
            selection_bg: colors.highlight,
            on_selection: colors.highlight_text,
            selection_mark: colors.highlight_text,
            focus: colors.window_text,
        }
    }

    /// I token del tema chiaro o scuro.
    pub const fn of(theme: Theme) -> Tokens {
        match theme {
            Theme::Light => Tokens::light(),
            Theme::Dark => Tokens::dark(),
        }
    }
}

/// Applica token, corpi e spaziature ai due temi del contesto e imposta la preferenza. Con
/// `contrast` (tema a contrasto elevato del sistema) i due temi usano i colori di sistema.
pub fn apply(ctx: &egui::Context, choice: ThemeChoice, contrast: Option<SystemContrast>) {
    let contrast = contrast.as_ref().map(Tokens::high_contrast);
    ctx.data_mut(|data| {
        if let Some(tokens) = contrast {
            data.insert_temp(contrast_id(), tokens);
        } else {
            data.remove::<Tokens>(contrast_id());
        }
    });
    for theme in [Theme::Light, Theme::Dark] {
        let tokens = contrast.unwrap_or_else(|| Tokens::of(theme));
        ctx.set_visuals_of(theme, visuals(&tokens, theme));
        ctx.style_mut_of(theme, |style| {
            style.text_styles = text_styles();
            style.spacing.item_spacing = egui::vec2(SPACE[1], SPACE[1]);
            style.spacing.button_padding = egui::vec2(SPACE[1], SPACE[0]);
            style.spacing.interact_size.y = INTERACT_HEIGHT;
            style.spacing.indent = SPACE[3];
        });
    }
    set_choice(ctx, choice);
}

/// Cambia solo la preferenza di tema (Sistema, Chiaro, Scuro).
pub fn set_choice(ctx: &egui::Context, choice: ThemeChoice) {
    ctx.set_theme(choice.preference());
}

/// I token del tema in uso ora nel contesto.
pub fn current(ctx: &egui::Context) -> Tokens {
    ctx.data(|data| data.get_temp::<Tokens>(contrast_id()))
        .unwrap_or_else(|| Tokens::of(ctx.theme()))
}

/// Dove il contesto conserva i token del contrasto elevato, se attivo.
fn contrast_id() -> egui::Id {
    egui::Id::unique("lynkeus-system-contrast")
}

/// Il bordo di focus della tastiera.
pub fn focus_stroke(tokens: &Tokens) -> Stroke {
    Stroke::new(FOCUS_WIDTH, tokens.focus)
}

fn text_styles() -> std::collections::BTreeMap<TextStyle, FontId> {
    [
        (TextStyle::Small, FontId::proportional(CAPTION_SIZE)),
        (TextStyle::Body, FontId::proportional(BODY_SIZE)),
        (TextStyle::Button, FontId::proportional(BODY_SIZE)),
        (TextStyle::Monospace, FontId::monospace(FONT[1])),
        (TextStyle::Heading, FontId::new(FONT[4], semibold_family())),
    ]
    .into()
}

fn visuals(t: &Tokens, theme: Theme) -> Visuals {
    let mut v = match theme {
        Theme::Light => Visuals::light(),
        Theme::Dark => Visuals::dark(),
    };
    let control = CornerRadius::same(RADIUS[0]);
    let overlay = CornerRadius::same(RADIUS[1]);

    v.override_text_color = None;
    v.weak_text_color = Some(t.text_weak);
    v.hyperlink_color = t.accent;
    v.panel_fill = t.bg;
    v.window_fill = t.bg_overlay;
    v.window_stroke = Stroke::new(1.0, t.stroke_weak);
    v.extreme_bg_color = t.bg_raised;
    v.text_edit_bg_color = Some(t.bg_raised);
    v.faint_bg_color = t.fill;
    v.code_bg_color = t.fill;
    v.error_fg_color = t.error;
    v.warn_fg_color = t.warning;
    v.window_corner_radius = overlay;
    v.menu_corner_radius = overlay;
    v.selection.bg_fill = t.selection_bg;
    // Testo delle righe selezionate; i campi di testo lo usano anche come bordo di focus
    // (il modulo lo sostituisce con `focus_stroke`, vedi `ui::search_panel`).
    v.selection.stroke = Stroke::new(FOCUS_WIDTH, t.on_selection);
    v.text_cursor.stroke = Stroke::new(2.0, t.accent);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = t.bg;
    w.noninteractive.weak_bg_fill = t.bg;
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.stroke_weak);
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.text_strong);
    w.noninteractive.corner_radius = control;

    w.inactive.bg_fill = t.fill;
    w.inactive.weak_bg_fill = t.fill;
    w.inactive.bg_stroke = Stroke::new(1.0, t.stroke_strong);
    w.inactive.fg_stroke = Stroke::new(1.0, t.text_strong);
    w.inactive.corner_radius = control;

    w.hovered.bg_fill = t.fill_hover;
    w.hovered.weak_bg_fill = t.fill_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, t.text_weak);
    w.hovered.fg_stroke = Stroke::new(1.5, t.text_strong);
    w.hovered.corner_radius = control;

    // Focus e pressione: bordo di 2 px che contrasta con ogni fondo (anche con l'accento
    // del primario, dove l'anello si disegna all'esterno).
    w.active.bg_fill = t.fill_hover;
    w.active.weak_bg_fill = t.fill_hover;
    w.active.bg_stroke = focus_stroke(t);
    w.active.fg_stroke = Stroke::new(2.0, t.text_strong);
    w.active.corner_radius = control;

    w.open.bg_fill = t.fill_hover;
    w.open.weak_bg_fill = t.fill_hover;
    w.open.bg_stroke = Stroke::new(1.0, t.stroke_strong);
    w.open.fg_stroke = Stroke::new(1.0, t.text_strong);
    w.open.corner_radius = control;
    v
}

/// Rapporto di contrasto WCAG 2 tra due colori opachi (da 1 a 21).
pub fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (light, dark) = if la >= lb { (la, lb) } else { (lb, la) };
    (light + 0.05) / (dark + 0.05)
}

/// Luminanza relativa WCAG di un colore sRGB.
fn relative_luminance(color: Color32) -> f32 {
    let channel = |c: u8| {
        let c = f32::from(c) / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT_MIN: f32 = 4.5;
    const NON_TEXT_MIN: f32 = 3.0;

    fn both() -> [(&'static str, Tokens); 2] {
        [("chiaro", Tokens::light()), ("scuro", Tokens::dark())]
    }

    #[track_caller]
    fn assert_contrast(name: &str, pair: &str, a: Color32, b: Color32, min: f32) {
        let ratio = contrast_ratio(a, b);
        assert!(ratio >= min, "{name}, {pair}: {ratio:.2} < {min}");
    }

    #[test]
    fn known_ratios() {
        let ratio = contrast_ratio(Color32::BLACK, Color32::WHITE);
        assert!((ratio - 21.0).abs() < 0.01, "{ratio}");
        assert!((contrast_ratio(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn text_has_at_least_4_5_to_1() {
        for (name, t) in both() {
            for (bg_name, bg) in [
                ("bg", t.bg),
                ("bg_raised", t.bg_raised),
                ("bg_overlay", t.bg_overlay),
                ("fill", t.fill),
                ("fill_hover", t.fill_hover),
                ("selection_bg", t.selection_bg),
            ] {
                assert_contrast(
                    name,
                    &format!("text_strong/{bg_name}"),
                    t.text_strong,
                    bg,
                    TEXT_MIN,
                );
            }
            for (bg_name, bg) in [("bg", t.bg), ("bg_raised", t.bg_raised), ("fill", t.fill)] {
                assert_contrast(
                    name,
                    &format!("text_weak/{bg_name}"),
                    t.text_weak,
                    bg,
                    TEXT_MIN,
                );
                assert_contrast(name, &format!("error/{bg_name}"), t.error, bg, TEXT_MIN);
                assert_contrast(name, &format!("warning/{bg_name}"), t.warning, bg, TEXT_MIN);
                assert_contrast(name, &format!("link/{bg_name}"), t.accent, bg, TEXT_MIN);
            }
            assert_contrast(name, "on_accent/accent", t.on_accent, t.accent, TEXT_MIN);
            assert_contrast(
                name,
                "text_strong/highlight_bg",
                t.text_strong,
                t.highlight_bg,
                TEXT_MIN,
            );
        }
    }

    #[test]
    fn controls_and_borders_have_at_least_3_to_1() {
        for (name, t) in both() {
            for (bg_name, bg) in [("bg", t.bg), ("bg_raised", t.bg_raised)] {
                assert_contrast(
                    name,
                    &format!("stroke_strong/{bg_name}"),
                    t.stroke_strong,
                    bg,
                    NON_TEXT_MIN,
                );
                assert_contrast(
                    name,
                    &format!("accent/{bg_name}"),
                    t.accent,
                    bg,
                    NON_TEXT_MIN,
                );
            }
        }
    }

    /// T-C07 (C-02): la riga selezionata non si riconosce solo dal fondo: il segno sul
    /// bordo sinistro ha almeno 3:1 contro il fondo della selezione e contro ogni fondo di
    /// riga (normale, alterna, sotto il puntatore).
    #[test]
    fn selection_contrast() {
        for (name, t) in both() {
            for (bg_name, bg) in [
                ("selection_bg", t.selection_bg),
                ("bg_raised", t.bg_raised),
                ("fill", t.fill),
                ("fill_hover", t.fill_hover),
            ] {
                assert_contrast(
                    name,
                    &format!("selection_mark/{bg_name}"),
                    t.selection_mark,
                    bg,
                    NON_TEXT_MIN,
                );
            }
            assert_contrast(
                name,
                "on_selection/selection_bg",
                t.on_selection,
                t.selection_bg,
                TEXT_MIN,
            );
        }
    }

    /// T-C11 (C-09): il bordo di focus è largo 2 px e ha almeno 3:1 contro tutti i fondi
    /// su cui compare (pannello, campi, controlli, primario disegnato all'esterno).
    #[test]
    fn focus_ring_contrast() {
        for (name, t) in both() {
            for (bg_name, bg) in [
                ("bg", t.bg),
                ("bg_raised", t.bg_raised),
                ("bg_overlay", t.bg_overlay),
                ("fill", t.fill),
                ("fill_hover", t.fill_hover),
            ] {
                assert_contrast(name, &format!("focus/{bg_name}"), t.focus, bg, NON_TEXT_MIN);
            }
            let theme = if name == "chiaro" {
                Theme::Light
            } else {
                Theme::Dark
            };
            let v = visuals(&t, theme);
            assert_eq!(v.widgets.active.bg_stroke, focus_stroke(&t), "{name}");
            assert!(focus_stroke(&t).width >= 2.0);
        }
    }

    /// Le tavolozze dei temi a contrasto elevato di Windows: Nero e Bianco classici, più
    /// Acquatico e Deserto di Windows 11.
    fn windows_contrast_themes() -> [(&'static str, SystemContrast); 4] {
        let rgb = Color32::from_rgb;
        [
            (
                "nero",
                SystemContrast {
                    window: rgb(0x00, 0x00, 0x00),
                    window_text: rgb(0xFF, 0xFF, 0xFF),
                    highlight: rgb(0x1A, 0xEB, 0xFF),
                    highlight_text: rgb(0x00, 0x00, 0x00),
                    button_face: rgb(0x00, 0x00, 0x00),
                },
            ),
            (
                "bianco",
                SystemContrast {
                    window: rgb(0xFF, 0xFF, 0xFF),
                    window_text: rgb(0x00, 0x00, 0x00),
                    highlight: rgb(0x37, 0x00, 0x6E),
                    highlight_text: rgb(0xFF, 0xFF, 0xFF),
                    button_face: rgb(0xFF, 0xFF, 0xFF),
                },
            ),
            (
                "acquatico",
                SystemContrast {
                    window: rgb(0x20, 0x20, 0x20),
                    window_text: rgb(0xFF, 0xFF, 0xFF),
                    highlight: rgb(0x8E, 0xE3, 0xF0),
                    highlight_text: rgb(0x26, 0x3B, 0x50),
                    button_face: rgb(0x20, 0x20, 0x20),
                },
            ),
            (
                "deserto",
                SystemContrast {
                    window: rgb(0xFF, 0xFA, 0xEF),
                    window_text: rgb(0x3D, 0x3D, 0x3D),
                    highlight: rgb(0x90, 0x39, 0x09),
                    highlight_text: rgb(0xFF, 0xF5, 0xE3),
                    button_face: rgb(0xFF, 0xFA, 0xEF),
                },
            ),
        ]
    }

    /// C-P4: con un tema a contrasto elevato i token vengono dai colori di sistema e le
    /// coppie usate dall'interfaccia restano leggibili.
    #[test]
    fn high_contrast_tokens_follow_the_system_colors() {
        for (name, colors) in windows_contrast_themes() {
            let t = Tokens::high_contrast(&colors);
            assert_eq!(t.bg, colors.window, "{name}");
            assert_eq!(t.text_strong, colors.window_text, "{name}");
            assert_eq!(t.selection_bg, colors.highlight, "{name}");
            assert_eq!(t.on_selection, colors.highlight_text, "{name}");
            for (pair, a, b, min) in [
                ("testo/fondo", t.text_strong, t.bg, TEXT_MIN),
                ("testo debole/fondo", t.text_weak, t.bg, TEXT_MIN),
                ("errore/fondo", t.error, t.bg, TEXT_MIN),
                ("testo/controlli", t.text_strong, t.fill, TEXT_MIN),
                ("selezione", t.on_selection, t.selection_bg, TEXT_MIN),
                ("primario", t.on_accent, t.accent, TEXT_MIN),
                ("corrispondenze", t.text_strong, t.highlight_bg, TEXT_MIN),
                ("bordi", t.stroke_strong, t.bg, NON_TEXT_MIN),
                ("focus", t.focus, t.bg, NON_TEXT_MIN),
                ("segno", t.selection_mark, t.selection_bg, NON_TEXT_MIN),
            ] {
                assert_contrast(name, pair, a, b, min);
            }
        }
    }

    /// L'accento è un blu: niente rosso né arancio, che si leggerebbero come errore o avviso.
    #[test]
    fn accent_is_neither_red_nor_orange() {
        for (name, t) in both() {
            let (r, b) = (t.accent.r(), t.accent.b());
            assert!(b > r, "{name}: accento non blu {:?}", t.accent);
        }
    }

    #[test]
    fn theme_choice_maps_to_egui() {
        assert_eq!(ThemeChoice::default().preference(), ThemePreference::System);
        assert_eq!(ThemeChoice::Dark.preference(), ThemePreference::Dark);
        assert_eq!(ThemeChoice::Light.preference(), ThemePreference::Light);
    }
}
