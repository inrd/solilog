//! Themes: built-in presets (`assets/themes/*.json`) plus optional user
//! themes in `~/.config/solilog/themes/*.json` (see [`user_themes_dir`]).
//!
//! A theme JSON file may specify `"extends": "dark"` (or `"mmx"`) to inherit
//! every field it omits. Comments and trailing commas are allowed.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use anyhow::{Context as _, Result, bail};
use gpui::{
    AnyElement, App, Bounds, BoxShadow, Div, Global, Hsla, IntoElement, Pixels, Styled, canvas,
    fill, point, px, rgb, rgba, size,
};
use serde::Deserialize;
use solilog_core::{LabelColor, ProjectStatus, Status};

use super::keymap;

pub const DEFAULT_THEME_ID: &str = "dark";

const BUILTIN_THEMES: &[(&str, &str)] = &[
    ("dark", include_str!("../../assets/themes/dark.json")),
    ("8086", include_str!("../../assets/themes/8086.json")),
    ("mmx", include_str!("../../assets/themes/mmx.json")),
    ("sse2", include_str!("../../assets/themes/sse2.json")),
    ("sse2-dark", include_str!("../../assets/themes/sse2-dark.json")),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Chrome {
    Flat,
    Bevel,
    Glass,
    Crt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitlebarStyle {
    Native,
    Classic,
    Glass,
    Crt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IconSet {
    Default,
    Pixel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowStyle {
    Soft,
    Hard,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusColors {
    pub backlog: Hsla,
    pub todo: Hsla,
    pub in_progress: Hsla,
    pub in_review: Hsla,
    pub blocked: Hsla,
    pub done: Hsla,
    pub cancelled: Hsla,
    pub glyph: Hsla,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelPalette {
    pub red: Hsla,
    pub orange: Hsla,
    pub yellow: Hsla,
    pub green: Hsla,
    pub teal: Hsla,
    pub blue: Hsla,
    pub purple: Hsla,
    pub pink: Hsla,
    pub gray: Hsla,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeColors {
    pub background: Hsla,
    pub sidebar: Hsla,
    pub surface: Hsla,
    pub input: Hsla,
    pub tooltip: Hsla,
    pub border: Hsla,
    pub divider: Hsla,
    pub hover: Hsla,
    pub highlight: Hsla,
    pub highlight_text: Hsla,
    pub selection: Hsla,
    pub selection_highlight: Hsla,
    pub selection_text: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub accent: Hsla,
    pub error: Hsla,
    pub review: Hsla,
    pub passed: Hsla,
    pub text_selection: Hsla,
    pub overlay: Hsla,
    pub bevel_light: Hsla,
    pub bevel_highlight: Hsla,
    pub bevel_shadow: Hsla,
    pub bevel_dark: Hsla,
    pub titlebar_active_start: Hsla,
    pub titlebar_active_end: Hsla,
    pub titlebar_inactive_start: Hsla,
    pub titlebar_inactive_end: Hsla,
    pub titlebar_text: Hsla,
    pub titlebar_inactive_text: Hsla,
    pub glass_highlight: Hsla,
    pub glass_glow: Hsla,
    pub status: StatusColors,
    pub labels: LabelPalette,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub appearance: Appearance,
    pub chrome: Chrome,
    pub titlebar: TitlebarStyle,
    pub icons: IconSet,
    pub radius: f32,
    pub shadow: ShadowStyle,
    pub font_family: Option<String>,
    pub colors: ThemeColors,
}

impl Theme {
    pub fn is_bevel(&self) -> bool {
        self.chrome == Chrome::Bevel
    }

    pub fn is_glass(&self) -> bool {
        self.chrome == Chrome::Glass
    }

    pub fn is_crt(&self) -> bool {
        self.chrome == Chrome::Crt
    }

    pub fn is_classic_titlebar(&self) -> bool {
        self.titlebar == TitlebarStyle::Classic
    }

    pub fn is_glass_titlebar(&self) -> bool {
        self.titlebar == TitlebarStyle::Glass
    }

    pub fn is_crt_titlebar(&self) -> bool {
        self.titlebar == TitlebarStyle::Crt
    }

    pub fn uses_custom_titlebar(&self) -> bool {
        self.titlebar != TitlebarStyle::Native
    }

    pub fn is_pixel_icons(&self) -> bool {
        self.icons == IconSet::Pixel
    }

    /// Resolves generic font aliases (like `"monospace"`) to a platform font
    /// family GPUI can load on both macOS and Linux.
    pub fn resolved_font_family(&self) -> Option<String> {
        let family = self.font_family.as_deref()?;
        if family.eq_ignore_ascii_case("monospace") {
            Some(if cfg!(target_os = "macos") {
                "Menlo".to_owned()
            } else {
                "monospace".to_owned()
            })
        } else {
            Some(family.to_owned())
        }
    }

    pub fn radius_sm(&self) -> Pixels {
        px(self.radius)
    }

    pub fn radius_md(&self) -> Pixels {
        px(if self.radius <= 0. {
            0.
        } else {
            self.radius + 2.
        })
    }

    pub fn label_color(&self, color: Option<LabelColor>) -> Hsla {
        use LabelColor::*;
        match color {
            None => self.colors.text_muted,
            Some(Red) => self.colors.labels.red,
            Some(Orange) => self.colors.labels.orange,
            Some(Yellow) => self.colors.labels.yellow,
            Some(Green) => self.colors.labels.green,
            Some(Teal) => self.colors.labels.teal,
            Some(Blue) => self.colors.labels.blue,
            Some(Purple) => self.colors.labels.purple,
            Some(Pink) => self.colors.labels.pink,
            Some(Gray) => self.colors.labels.gray,
        }
    }

    pub fn status_color(&self, status: Status) -> Hsla {
        match status {
            Status::Backlog => self.colors.status.backlog,
            Status::Todo => self.colors.status.todo,
            Status::InProgress => self.colors.status.in_progress,
            Status::InReview => self.colors.status.in_review,
            Status::Blocked => self.colors.status.blocked,
            Status::Done => self.colors.status.done,
            Status::Cancelled => self.colors.status.cancelled,
        }
    }

    pub fn project_status_color(&self, status: ProjectStatus) -> Hsla {
        match status {
            ProjectStatus::Planned => self.colors.status.todo,
            ProjectStatus::Active => self.colors.status.in_progress,
            ProjectStatus::Done => self.colors.status.done,
            ProjectStatus::Archived => self.colors.status.backlog,
        }
    }

    /// 2px Win95-style raised bevel overlay (outer light/dark, inner highlight/shadow).
    /// Returns `None` in flat themes.
    pub fn raised_bevel(&self) -> Option<AnyElement> {
        self.is_bevel().then(|| {
            bevel_canvas(
                self.colors.bevel_light,
                self.colors.bevel_dark,
                Some((self.colors.bevel_highlight, self.colors.bevel_shadow)),
            )
        })
    }

    /// 1px raised bevel overlay for compact buttons and chips.
    pub fn raised_bevel_thin(&self) -> Option<AnyElement> {
        self.is_bevel()
            .then(|| bevel_canvas(self.colors.bevel_light, self.colors.bevel_shadow, None))
    }

    /// 2px Win95-style sunken bevel overlay (inputs, checkboxes, list wells).
    pub fn sunken_bevel(&self) -> Option<AnyElement> {
        self.is_bevel().then(|| {
            bevel_canvas(
                self.colors.bevel_shadow,
                self.colors.bevel_light,
                Some((self.colors.bevel_dark, self.colors.bevel_highlight)),
            )
        })
    }

    /// 1px sunken bevel overlay for status bar cells and subtle wells.
    pub fn sunken_bevel_thin(&self) -> Option<AnyElement> {
        self.is_bevel()
            .then(|| bevel_canvas(self.colors.bevel_shadow, self.colors.bevel_light, None))
    }

    /// 1px dotted focus rectangle inset by 2px, for focused elements in bevel themes.
    pub fn dotted_focus(&self, color: Hsla) -> Option<AnyElement> {
        self.is_bevel().then(|| dotted_focus_canvas(color))
    }

    /// Vista / PSP-style two-tone top-half specular gloss sheen with a crisp
    /// 1px inner highlight rim. Returns `None` in non-glass themes.
    pub fn gloss_overlay(&self) -> Option<AnyElement> {
        self.is_glass()
            .then(|| glass_gloss_canvas(self.colors.glass_highlight, None))
    }

    /// Subtle horizontal glass reflection band for toolbars and headers.
    pub fn glass_bar_overlay(&self) -> Option<AnyElement> {
        if !self.is_glass() {
            return None;
        }
        let mut sheen = self.colors.glass_highlight;
        sheen.a *= 0.65;
        Some(glass_gloss_canvas(sheen, None))
    }

    /// Glossy selection highlight (top-half sheen + luminous outer glow rim)
    /// for active rows in lists, sidebars, and pickers.
    pub fn glass_selection_overlay(&self) -> Option<AnyElement> {
        if !self.is_glass() {
            return None;
        }
        let mut sheen = self.colors.glass_highlight;
        sheen.a *= 0.55;
        Some(glass_gloss_canvas(sheen, Some(self.colors.glass_glow)))
    }

    /// Ultra-subtle horizontal CRT scanlines (1px every 3px) for `"chrome": "crt"`.
    pub fn crt_scanlines_overlay(&self) -> Option<AnyElement> {
        self.is_crt().then(crt_scanlines_canvas)
    }

    /// Applies the theme's popup shadow (pickers, menus).
    pub fn apply_popup_shadow(&self, el: Div) -> Div {
        match self.shadow {
            ShadowStyle::Soft => el.shadow_lg(),
            ShadowStyle::Hard => el.shadow(vec![BoxShadow {
                color: rgba(0x00000099).into(),
                offset: point(px(3.), px(3.)),
                blur_radius: px(0.),
                spread_radius: px(0.),
            }]),
            ShadowStyle::None => el,
        }
    }

    /// Applies the theme's tooltip shadow.
    pub fn apply_tooltip_shadow(&self, el: Div) -> Div {
        match self.shadow {
            ShadowStyle::Soft => el.shadow_md(),
            ShadowStyle::Hard => el.shadow(vec![BoxShadow {
                color: rgba(0x00000080).into(),
                offset: point(px(2.), px(2.)),
                blur_radius: px(0.),
                spread_radius: px(0.),
            }]),
            ShadowStyle::None => el,
        }
    }
}

/// Paints a 1px or 2px 3D bevel along the inside edges of its parent container.
fn bevel_canvas(
    outer_tl: Hsla,
    outer_br: Hsla,
    inner: Option<(Hsla, Hsla)>,
) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            paint_bevel_ring(bounds, px(0.), outer_tl, outer_br, window);
            if let Some((inner_tl, inner_br)) = inner {
                paint_bevel_ring(bounds, px(1.), inner_tl, inner_br, window);
            }
        },
    )
    .absolute()
    .inset_0()
    .into_any_element()
}

fn paint_bevel_ring(
    bounds: Bounds<Pixels>,
    inset: Pixels,
    tl: Hsla,
    br: Hsla,
    window: &mut gpui::Window,
) {
    let x = bounds.origin.x + inset;
    let y = bounds.origin.y + inset;
    let w = (bounds.size.width - inset * 2.).max(px(0.));
    let h = (bounds.size.height - inset * 2.).max(px(0.));
    if w <= px(0.) || h <= px(0.) {
        return;
    }
    let one = px(1.);
    // Top edge
    window.paint_quad(fill(Bounds::new(point(x, y), size(w, one)), tl));
    // Left edge
    window.paint_quad(fill(Bounds::new(point(x, y), size(one, h)), tl));
    // Bottom edge
    window.paint_quad(fill(
        Bounds::new(point(x, y + h - one), size(w, one)),
        br,
    ));
    // Right edge
    window.paint_quad(fill(
        Bounds::new(point(x + w - one, y), size(one, h)),
        br,
    ));
}

/// Paints a glossy top-half specular reflection and 1px inner rim (plus an
/// optional luminous outer border) inside its parent container.
fn glass_gloss_canvas(sheen: Hsla, glow_border: Option<Hsla>) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            let w = bounds.size.width;
            let h = bounds.size.height;
            if w <= px(2.) || h <= px(2.) {
                return;
            }
            let x = bounds.origin.x;
            let y = bounds.origin.y;
            let one = px(1.);
            // Top-half specular gloss band (signature Vista / Aqua / PSP look).
            let half_h = (h * 0.48).max(one);
            window.paint_quad(fill(Bounds::new(point(x + one, y + one), size(w - one * 2., half_h)), sheen));
            // Crisp 1px top specular catch-light.
            let mut rim = sheen;
            rim.a = (rim.a * 1.4).min(0.85);
            window.paint_quad(fill(Bounds::new(point(x + one, y + one), size(w - one * 2., one)), rim));
            if let Some(glow) = glow_border {
                paint_bevel_ring(bounds, px(0.), glow, glow, window);
            }
        },
    )
    .absolute()
    .inset_0()
    .into_any_element()
}

/// Paints ultra-subtle horizontal 1px scanlines every 3px across a CRT surface.
fn crt_scanlines_canvas() -> AnyElement {
    let line_color: Hsla = rgba(0x00000038).into();
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            let x = bounds.origin.x;
            let w = bounds.size.width;
            let y_start = f32::from(bounds.origin.y).floor();
            let y_end = y_start + f32::from(bounds.size.height);
            let mut y = y_start;
            while y < y_end {
                window.paint_quad(fill(Bounds::new(point(x, px(y)), size(w, px(1.))), line_color));
                y += 3.;
            }
        },
    )
    .absolute()
    .inset_0()
    .into_any_element()
}

/// Paints a 1px alternating-pixel dotted rectangle inset by 2px.
fn dotted_focus_canvas(color: Hsla) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            let inset = px(2.);
            let x0 = f32::from(bounds.origin.x + inset).floor();
            let y0 = f32::from(bounds.origin.y + inset).floor();
            let w = (f32::from(bounds.size.width) - 4.).floor();
            let h = (f32::from(bounds.size.height) - 4.).floor();
            if w <= 2. || h <= 2. {
                return;
            }
            let x1 = x0 + w - 1.;
            let y1 = y0 + h - 1.;
            let dot = size(px(1.), px(1.));
            let mut x = x0;
            while x <= x1 {
                window.paint_quad(fill(Bounds::new(point(px(x), px(y0)), dot), color));
                window.paint_quad(fill(Bounds::new(point(px(x), px(y1)), dot), color));
                x += 2.;
            }
            let mut y = y0 + 2.;
            while y < y1 {
                window.paint_quad(fill(Bounds::new(point(px(x0), px(y)), dot), color));
                window.paint_quad(fill(Bounds::new(point(px(x1), px(y)), dot), color));
                y += 2.;
            }
        },
    )
    .absolute()
    .inset_0()
    .into_any_element()
}

// ---------------------------------------------------------------------------
// JSON deserialization (partial files inherit from their base theme)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTheme {
    name: Option<String>,
    extends: Option<String>,
    appearance: Option<Appearance>,
    chrome: Option<Chrome>,
    titlebar: Option<TitlebarStyle>,
    icons: Option<IconSet>,
    radius: Option<f32>,
    shadow: Option<ShadowStyle>,
    font_family: Option<String>,
    #[serde(default)]
    colors: RawColors,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawColors {
    background: Option<String>,
    sidebar: Option<String>,
    surface: Option<String>,
    input: Option<String>,
    tooltip: Option<String>,
    border: Option<String>,
    divider: Option<String>,
    hover: Option<String>,
    highlight: Option<String>,
    highlight_text: Option<String>,
    selection: Option<String>,
    selection_highlight: Option<String>,
    selection_text: Option<String>,
    text: Option<String>,
    text_muted: Option<String>,
    accent: Option<String>,
    error: Option<String>,
    review: Option<String>,
    passed: Option<String>,
    text_selection: Option<String>,
    overlay: Option<String>,
    bevel_light: Option<String>,
    bevel_highlight: Option<String>,
    bevel_shadow: Option<String>,
    bevel_dark: Option<String>,
    titlebar_active_start: Option<String>,
    titlebar_active_end: Option<String>,
    titlebar_inactive_start: Option<String>,
    titlebar_inactive_end: Option<String>,
    titlebar_text: Option<String>,
    titlebar_inactive_text: Option<String>,
    glass_highlight: Option<String>,
    glass_glow: Option<String>,
    #[serde(default)]
    status: RawStatusColors,
    #[serde(default)]
    labels: RawLabelPalette,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStatusColors {
    backlog: Option<String>,
    todo: Option<String>,
    in_progress: Option<String>,
    in_review: Option<String>,
    blocked: Option<String>,
    done: Option<String>,
    cancelled: Option<String>,
    glyph: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLabelPalette {
    red: Option<String>,
    orange: Option<String>,
    yellow: Option<String>,
    green: Option<String>,
    teal: Option<String>,
    blue: Option<String>,
    purple: Option<String>,
    pink: Option<String>,
    gray: Option<String>,
}

/// Parses `#rgb`, `#rrggbb`, or `#rrggbbaa` into an `Hsla` color.
pub fn parse_hex_color(input: &str) -> Result<Hsla> {
    let hex = input
        .trim()
        .strip_prefix('#')
        .with_context(|| format!("color {input:?} must start with '#'"))?;
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16)? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16)? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16)? * 17;
            let value = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
            Ok(rgb(value).into())
        }
        6 => {
            let value = u32::from_str_radix(hex, 16)
                .with_context(|| format!("invalid hex color {input:?}"))?;
            Ok(rgb(value).into())
        }
        8 => {
            let value = u32::from_str_radix(hex, 16)
                .with_context(|| format!("invalid hex color {input:?}"))?;
            Ok(rgba(value).into())
        }
        _ => bail!("invalid hex color {input:?}: expected #rgb, #rrggbb, or #rrggbbaa"),
    }
}

fn apply_color(target: &mut Hsla, raw: Option<&str>, field: &str) -> Result<()> {
    if let Some(s) = raw {
        *target = parse_hex_color(s).with_context(|| format!("colors.{field}"))?;
    }
    Ok(())
}

fn default_dark_theme() -> Theme {
    parse_complete("dark", BUILTIN_THEMES[0].1).expect("built-in dark.json must be valid")
}

fn parse_complete(id: &str, source: &str) -> Result<Theme> {
    let raw: RawTheme =
        serde_json_lenient::from_str_lenient(source).context("invalid theme JSON")?;
    let mut base = Theme {
        id: id.to_owned(),
        name: raw.name.clone().unwrap_or_else(|| id.to_owned()),
        appearance: raw.appearance.context("missing 'appearance'")?,
        chrome: raw.chrome.unwrap_or(Chrome::Flat),
        titlebar: raw.titlebar.unwrap_or(TitlebarStyle::Native),
        icons: raw.icons.unwrap_or(IconSet::Default),
        radius: raw.radius.unwrap_or(4.),
        shadow: raw.shadow.unwrap_or(ShadowStyle::Soft),
        font_family: raw.font_family.clone(),
        colors: ThemeColors {
            background: rgb(0).into(),
            sidebar: rgb(0).into(),
            surface: rgb(0).into(),
            input: rgb(0).into(),
            tooltip: rgb(0).into(),
            border: rgb(0).into(),
            divider: rgb(0).into(),
            hover: rgb(0).into(),
            highlight: rgb(0).into(),
            highlight_text: rgb(0).into(),
            selection: rgb(0).into(),
            selection_highlight: rgb(0).into(),
            selection_text: rgb(0).into(),
            text: rgb(0).into(),
            text_muted: rgb(0).into(),
            accent: rgb(0).into(),
            error: rgb(0).into(),
            review: rgb(0).into(),
            passed: rgb(0).into(),
            text_selection: rgb(0).into(),
            overlay: rgb(0).into(),
            bevel_light: rgb(0xffffff).into(),
            bevel_highlight: rgb(0xdfdfdf).into(),
            bevel_shadow: rgb(0x808080).into(),
            bevel_dark: rgb(0x000000).into(),
            titlebar_active_start: rgb(0x19191c).into(),
            titlebar_active_end: rgb(0x19191c).into(),
            titlebar_inactive_start: rgb(0x19191c).into(),
            titlebar_inactive_end: rgb(0x19191c).into(),
            titlebar_text: rgb(0x7a7a85).into(),
            titlebar_inactive_text: rgb(0x5a5a64).into(),
            glass_highlight: rgba(0xffffff20).into(),
            glass_glow: rgba(0x7c8cff66).into(),
            status: StatusColors {
                backlog: rgb(0).into(),
                todo: rgb(0).into(),
                in_progress: rgb(0).into(),
                in_review: rgb(0).into(),
                blocked: rgb(0).into(),
                done: rgb(0).into(),
                cancelled: rgb(0).into(),
                glyph: rgb(0).into(),
            },
            labels: LabelPalette {
                red: rgb(0).into(),
                orange: rgb(0).into(),
                yellow: rgb(0).into(),
                green: rgb(0).into(),
                teal: rgb(0).into(),
                blue: rgb(0).into(),
                purple: rgb(0).into(),
                pink: rgb(0).into(),
                gray: rgb(0).into(),
            },
        },
    };
    merge_raw(&mut base, &raw)?;
    Ok(base)
}

/// Parses a theme JSON file, inheriting any omitted fields from its `extends`
/// base theme (or the built-in `dark` theme by default).
pub fn parse_theme(id: &str, source: &str, builtins: &[Theme]) -> Result<Theme> {
    let raw: RawTheme =
        serde_json_lenient::from_str_lenient(source).context("invalid theme JSON")?;
    let base_id = raw.extends.as_deref().unwrap_or(DEFAULT_THEME_ID);
    let mut theme = builtins
        .iter()
        .find(|t| t.id.eq_ignore_ascii_case(base_id))
        .cloned()
        .with_context(|| format!("unknown base theme {base_id:?} in 'extends'"))?;
    theme.id = id.to_owned();
    theme.name = raw.name.clone().unwrap_or_else(|| id.to_owned());
    merge_raw(&mut theme, &raw)?;
    Ok(theme)
}

fn merge_raw(theme: &mut Theme, raw: &RawTheme) -> Result<()> {
    if let Some(a) = raw.appearance {
        theme.appearance = a;
    }
    if let Some(c) = raw.chrome {
        theme.chrome = c;
    }
    if let Some(tb) = raw.titlebar {
        theme.titlebar = tb;
    }
    if let Some(ic) = raw.icons {
        theme.icons = ic;
    }
    if let Some(r) = raw.radius {
        theme.radius = r.max(0.);
    }
    if let Some(s) = raw.shadow {
        theme.shadow = s;
    }
    if let Some(ref f) = raw.font_family {
        theme.font_family = if f.trim().is_empty() {
            None
        } else {
            Some(f.clone())
        };
    }

    let c = &mut theme.colors;
    let rc = &raw.colors;
    apply_color(&mut c.background, rc.background.as_deref(), "background")?;
    apply_color(&mut c.sidebar, rc.sidebar.as_deref(), "sidebar")?;
    apply_color(&mut c.surface, rc.surface.as_deref(), "surface")?;
    apply_color(&mut c.input, rc.input.as_deref(), "input")?;
    apply_color(&mut c.tooltip, rc.tooltip.as_deref(), "tooltip")?;
    apply_color(&mut c.border, rc.border.as_deref(), "border")?;
    apply_color(&mut c.divider, rc.divider.as_deref(), "divider")?;
    apply_color(&mut c.hover, rc.hover.as_deref(), "hover")?;
    apply_color(&mut c.highlight, rc.highlight.as_deref(), "highlight")?;
    apply_color(
        &mut c.highlight_text,
        rc.highlight_text.as_deref(),
        "highlight_text",
    )?;
    apply_color(&mut c.selection, rc.selection.as_deref(), "selection")?;
    apply_color(
        &mut c.selection_highlight,
        rc.selection_highlight.as_deref(),
        "selection_highlight",
    )?;
    apply_color(
        &mut c.selection_text,
        rc.selection_text.as_deref(),
        "selection_text",
    )?;
    apply_color(&mut c.text, rc.text.as_deref(), "text")?;
    apply_color(&mut c.text_muted, rc.text_muted.as_deref(), "text_muted")?;
    apply_color(&mut c.accent, rc.accent.as_deref(), "accent")?;
    apply_color(&mut c.error, rc.error.as_deref(), "error")?;
    apply_color(&mut c.review, rc.review.as_deref(), "review")?;
    apply_color(&mut c.passed, rc.passed.as_deref(), "passed")?;
    apply_color(
        &mut c.text_selection,
        rc.text_selection.as_deref(),
        "text_selection",
    )?;
    apply_color(&mut c.overlay, rc.overlay.as_deref(), "overlay")?;
    apply_color(&mut c.bevel_light, rc.bevel_light.as_deref(), "bevel_light")?;
    apply_color(
        &mut c.bevel_highlight,
        rc.bevel_highlight.as_deref(),
        "bevel_highlight",
    )?;
    apply_color(
        &mut c.bevel_shadow,
        rc.bevel_shadow.as_deref(),
        "bevel_shadow",
    )?;
    apply_color(&mut c.bevel_dark, rc.bevel_dark.as_deref(), "bevel_dark")?;
    apply_color(
        &mut c.titlebar_active_start,
        rc.titlebar_active_start.as_deref(),
        "titlebar_active_start",
    )?;
    apply_color(
        &mut c.titlebar_active_end,
        rc.titlebar_active_end.as_deref(),
        "titlebar_active_end",
    )?;
    apply_color(
        &mut c.titlebar_inactive_start,
        rc.titlebar_inactive_start.as_deref(),
        "titlebar_inactive_start",
    )?;
    apply_color(
        &mut c.titlebar_inactive_end,
        rc.titlebar_inactive_end.as_deref(),
        "titlebar_inactive_end",
    )?;
    apply_color(
        &mut c.titlebar_text,
        rc.titlebar_text.as_deref(),
        "titlebar_text",
    )?;
    apply_color(
        &mut c.titlebar_inactive_text,
        rc.titlebar_inactive_text.as_deref(),
        "titlebar_inactive_text",
    )?;
    apply_color(
        &mut c.glass_highlight,
        rc.glass_highlight.as_deref(),
        "glass_highlight",
    )?;
    apply_color(&mut c.glass_glow, rc.glass_glow.as_deref(), "glass_glow")?;

    let s = &mut c.status;
    let rs = &rc.status;
    apply_color(&mut s.backlog, rs.backlog.as_deref(), "status.backlog")?;
    apply_color(&mut s.todo, rs.todo.as_deref(), "status.todo")?;
    apply_color(
        &mut s.in_progress,
        rs.in_progress.as_deref(),
        "status.in_progress",
    )?;
    apply_color(&mut s.in_review, rs.in_review.as_deref(), "status.in_review")?;
    apply_color(&mut s.blocked, rs.blocked.as_deref(), "status.blocked")?;
    apply_color(&mut s.done, rs.done.as_deref(), "status.done")?;
    apply_color(&mut s.cancelled, rs.cancelled.as_deref(), "status.cancelled")?;
    apply_color(&mut s.glyph, rs.glyph.as_deref(), "status.glyph")?;

    let l = &mut c.labels;
    let rl = &rc.labels;
    apply_color(&mut l.red, rl.red.as_deref(), "labels.red")?;
    apply_color(&mut l.orange, rl.orange.as_deref(), "labels.orange")?;
    apply_color(&mut l.yellow, rl.yellow.as_deref(), "labels.yellow")?;
    apply_color(&mut l.green, rl.green.as_deref(), "labels.green")?;
    apply_color(&mut l.teal, rl.teal.as_deref(), "labels.teal")?;
    apply_color(&mut l.blue, rl.blue.as_deref(), "labels.blue")?;
    apply_color(&mut l.purple, rl.purple.as_deref(), "labels.purple")?;
    apply_color(&mut l.pink, rl.pink.as_deref(), "labels.pink")?;
    apply_color(&mut l.gray, rl.gray.as_deref(), "labels.gray")?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Global registry & hot reload
// ---------------------------------------------------------------------------

/// Where user themes live: `$SOLILOG_CONFIG_DIR/themes`, else
/// `$XDG_CONFIG_HOME/solilog/themes`, else `~/.config/solilog/themes`.
pub fn user_themes_dir() -> Option<PathBuf> {
    keymap::config_dir().map(|dir| dir.join("themes"))
}

/// Ensures the user themes directory exists and returns its path.
pub fn ensure_user_themes_dir() -> Result<PathBuf> {
    let dir = user_themes_dir().context("no home directory")?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[derive(Clone, Debug)]
pub struct ThemeEntry {
    pub theme: Rc<Theme>,
    pub is_user: bool,
}

struct ThemeState {
    active_id: String,
    active: Rc<Theme>,
    entries: Vec<ThemeEntry>,
    user_stamp: Vec<(PathBuf, Option<SystemTime>)>,
}

impl Global for ThemeState {}

fn load_builtins() -> Vec<Theme> {
    let mut builtins = vec![default_dark_theme()];
    for &(id, source) in &BUILTIN_THEMES[1..] {
        match parse_theme(id, source, &builtins) {
            Ok(theme) => builtins.push(theme),
            Err(err) => eprintln!("built-in theme {id:?}: {err:#}"),
        }
    }
    builtins
}

fn scan_user_dir(dir: &Path) -> Vec<(PathBuf, Option<SystemTime>)> {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<_> = read_dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .map(|p| {
            let mtime = std::fs::metadata(&p).ok().and_then(|m| m.modified().ok());
            (p, mtime)
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn load_all() -> (Vec<ThemeEntry>, Vec<(PathBuf, Option<SystemTime>)>) {
    let builtins = load_builtins();
    let mut entries: Vec<ThemeEntry> = builtins
        .iter()
        .cloned()
        .map(|t| ThemeEntry {
            theme: Rc::new(t),
            is_user: false,
        })
        .collect();

    let stamp = user_themes_dir()
        .map(|dir| scan_user_dir(&dir))
        .unwrap_or_default();

    for (path, _) in &stamp {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let id = stem.to_ascii_lowercase();
        match std::fs::read_to_string(path) {
            Ok(source) => match parse_theme(&id, &source, &builtins) {
                Ok(theme) => {
                    let entry = ThemeEntry {
                        theme: Rc::new(theme),
                        is_user: true,
                    };
                    if let Some(existing) = entries
                        .iter_mut()
                        .find(|e| e.theme.id.eq_ignore_ascii_case(&id))
                    {
                        *existing = entry;
                    } else {
                        entries.push(entry);
                    }
                }
                Err(err) => eprintln!("{}: {err:#}", path.display()),
            },
            Err(err) => eprintln!("reading {}: {err}", path.display()),
        }
    }

    (entries, stamp)
}

fn resolve_active(entries: &[ThemeEntry], wanted: Option<&str>) -> Rc<Theme> {
    if let Some(id) = wanted
        && let Some(entry) = entries
            .iter()
            .find(|e| e.theme.id.eq_ignore_ascii_case(id) || e.theme.name.eq_ignore_ascii_case(id))
    {
        return entry.theme.clone();
    }
    entries
        .first()
        .map(|e| e.theme.clone())
        .unwrap_or_else(|| Rc::new(default_dark_theme()))
}

/// Initializes the theme registry and activates `selected_id` (falling back to
/// `"dark"` if missing or unknown).
pub fn init(selected_id: Option<&str>, cx: &mut App) {
    let (entries, user_stamp) = load_all();
    let active = resolve_active(&entries, selected_id);
    apply_window_chrome(&active);
    cx.set_global(ThemeState {
        active_id: active.id.clone(),
        active,
        entries,
        user_stamp,
    });
}

/// The currently active theme.
pub fn current(cx: &App) -> Rc<Theme> {
    cx.global::<ThemeState>().active.clone()
}

/// All available themes (built-ins + user themes).
pub fn entries(cx: &App) -> Vec<ThemeEntry> {
    cx.global::<ThemeState>().entries.clone()
}

/// Switches to a theme by id or display name.
pub fn select(id_or_name: &str, cx: &mut App) -> Result<Rc<Theme>> {
    let state = cx.global_mut::<ThemeState>();
    let found = state
        .entries
        .iter()
        .find(|e| {
            e.theme.id.eq_ignore_ascii_case(id_or_name)
                || e.theme.name.eq_ignore_ascii_case(id_or_name)
        })
        .map(|e| e.theme.clone())
        .with_context(|| format!("unknown theme {id_or_name:?}"))?;
    state.active_id = found.id.clone();
    state.active = found.clone();
    apply_window_chrome(&found);
    Ok(found)
}

/// Reloads user themes if files in `~/.config/solilog/themes` changed.
/// Returns `true` when reloaded.
pub fn reload_if_changed(cx: &mut App) -> bool {
    let stamp = user_themes_dir()
        .map(|dir| scan_user_dir(&dir))
        .unwrap_or_default();
    if cx.global::<ThemeState>().user_stamp == stamp {
        return false;
    }
    let active_id = cx.global::<ThemeState>().active_id.clone();
    let (entries, user_stamp) = load_all();
    let active = resolve_active(&entries, Some(&active_id));
    apply_window_chrome(&active);
    cx.set_global(ThemeState {
        active_id: active.id.clone(),
        active,
        entries,
        user_stamp,
    });
    true
}

/// Applies OS-level window chrome (macOS dark/light appearance and hiding
/// native traffic lights when a theme draws its own classic titlebar).
pub fn apply_window_chrome(theme: &Theme) {
    apply_appearance(theme.appearance);
    apply_native_window_buttons(!theme.uses_custom_titlebar());
}

/// Matches the native macOS window chrome (title bar, scrollbars) to the
/// active theme's light/dark appearance.
#[cfg(target_os = "macos")]
pub fn apply_appearance(appearance: Appearance) {
    use objc2_app_kit::{
        NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication,
    };

    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    let name = match appearance {
        Appearance::Dark => unsafe { NSAppearanceNameDarkAqua },
        Appearance::Light => unsafe { NSAppearanceNameAqua },
    };
    let ns = NSAppearance::appearanceNamed(name);
    NSApplication::sharedApplication(mtm).setAppearance(ns.as_deref());
}

#[cfg(not(target_os = "macos"))]
pub fn apply_appearance(_appearance: Appearance) {}

/// Shows or hides the macOS traffic-light buttons on every open window so
/// themes with `"titlebar": "classic"` can draw their own window buttons.
#[cfg(target_os = "macos")]
pub fn apply_native_window_buttons(visible: bool) {
    use objc2_app_kit::{NSApplication, NSWindowButton};

    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    for window in app.windows() {
        for kind in [
            NSWindowButton::CloseButton,
            NSWindowButton::MiniaturizeButton,
            NSWindowButton::ZoomButton,
        ] {
            if let Some(button) = window.standardWindowButton(kind) {
                button.setHidden(!visible);
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn apply_native_window_buttons(_visible: bool) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_themes_are_valid() {
        let builtins = load_builtins();
        assert_eq!(builtins.len(), 5);
        assert_eq!(builtins[0].id, "dark");
        assert_eq!(builtins[0].appearance, Appearance::Dark);
        assert_eq!(builtins[0].chrome, Chrome::Flat);

        assert_eq!(builtins[1].id, "8086");
        assert_eq!(builtins[1].appearance, Appearance::Dark);
        assert_eq!(builtins[1].chrome, Chrome::Crt);
        assert_eq!(builtins[1].titlebar, TitlebarStyle::Crt);
        assert_eq!(builtins[1].radius, 0.);

        assert_eq!(builtins[2].id, "mmx");
        assert_eq!(builtins[2].appearance, Appearance::Light);
        assert_eq!(builtins[2].chrome, Chrome::Bevel);
        assert_eq!(builtins[2].radius, 0.);
        assert_eq!(builtins[2].shadow, ShadowStyle::Hard);

        assert_eq!(builtins[3].id, "sse2");
        assert_eq!(builtins[3].appearance, Appearance::Light);
        assert_eq!(builtins[3].chrome, Chrome::Glass);
        assert_eq!(builtins[3].titlebar, TitlebarStyle::Glass);

        assert_eq!(builtins[4].id, "sse2-dark");
        assert_eq!(builtins[4].appearance, Appearance::Dark);
        assert_eq!(builtins[4].chrome, Chrome::Glass);
        assert_eq!(builtins[4].titlebar, TitlebarStyle::Glass);
    }

    #[test]
    fn partial_theme_inherits_from_base() {
        let builtins = load_builtins();
        let custom = parse_theme(
            "hotdog",
            r##"{
                // Hot Dog Stand inspired override
                "name": "Hot Dog Stand",
                "extends": "mmx",
                "colors": {
                    "surface": "#ff0000",
                    "highlight": "#ffff00",
                }
            }"##,
            &builtins,
        )
        .unwrap();
        assert_eq!(custom.name, "Hot Dog Stand");
        assert_eq!(custom.chrome, Chrome::Bevel);
        assert_eq!(custom.radius, 0.);
        assert_eq!(custom.colors.surface, rgb(0xff0000).into());
        assert_eq!(custom.colors.highlight, rgb(0xffff00).into());
        // Inherited from mmx:
        assert_eq!(custom.colors.background, builtins[2].colors.background);
    }

    #[test]
    fn hex_color_parsing() {
        assert_eq!(parse_hex_color("#fff").unwrap(), rgb(0xffffff).into());
        assert_eq!(parse_hex_color("#1c1c1f").unwrap(), rgb(0x1c1c1f).into());
        assert_eq!(
            parse_hex_color("#7c8cff40").unwrap(),
            rgba(0x7c8cff40).into()
        );
        assert!(parse_hex_color("1c1c1f").is_err());
        assert!(parse_hex_color("#12345").is_err());
    }
}
