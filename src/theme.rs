//! The three app themes (dark, dimmed blue-grey, light) and the dimmed theme's accent.
//! Dark and light are gpui-component's defaults; dimmed is built here.

use std::rc::Rc;

use gpui_kit::App;
use gpui_kit::component::{Theme, ThemeConfig, ThemeRegistry};
use serde::{Deserialize, Serialize};

use crate::settings::Settings;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    Dark,
    Dimmed,
    Light,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::Dark, ThemeChoice::Dimmed, ThemeChoice::Light];
}

/// The five accents offered by the dimmed theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accent {
    #[default]
    Blue,
    Violet,
    Teal,
    Amber,
    Rose,
}

impl Accent {
    pub const ALL: [Accent; 5] = [Accent::Blue, Accent::Violet, Accent::Teal, Accent::Amber, Accent::Rose];

    /// 0xRRGGBB
    pub fn rgb(self) -> u32 {
        match self {
            Accent::Blue => 0x4C8DFF,
            Accent::Violet => 0x9D7BFF,
            Accent::Teal => 0x2EC4B6,
            Accent::Amber => 0xF5A524,
            Accent::Rose => 0xF25C80,
        }
    }
}

/// Applies the theme in `Settings` to every window.
pub fn apply(cx: &mut App) {
    let settings = Settings::get(cx);
    let config = match settings.theme {
        ThemeChoice::Dark => ThemeRegistry::global(cx).default_dark_theme().clone(),
        ThemeChoice::Light => ThemeRegistry::global(cx).default_light_theme().clone(),
        ThemeChoice::Dimmed => Rc::new(dimmed(settings.accent)),
    };
    Theme::update(cx, |theme| theme.apply_config(&config));
}

/// Blue-grey dark theme, in gpui-component's theme file format. Colors that are not
/// listed fall back to what gpui-component derives from the listed ones.
fn dimmed(accent: Accent) -> ThemeConfig {
    let a = format!("#{:06x}", accent.rgb());
    let json = format!(
        r##"{{
        "name": "Dimmed", "mode": "dark",
        "colors": {{
            "background": "#22272e", "foreground": "#adbac7", "border": "#444c56", "caret": "#cdd9e5",
            "accent.background": "#2d333b", "accent.foreground": "#cdd9e5", "accordion.background": "#22272e",
            "muted.background": "#2d333b", "muted.foreground": "#768390",
            "popover.background": "#2d333b", "popover.foreground": "#adbac7",
            "primary.background": "{a}", "primary.foreground": "#ffffff",
            "primary.hover.background": "{a}e6", "primary.active.background": "{a}cc",
            "secondary.background": "#373e47", "secondary.foreground": "#cdd9e5",
            "secondary.hover.background": "#444c56", "secondary.active.background": "#2d333b",
            "input.border": "#444c56", "ring": "{a}", "selection.background": "{a}66",
            "link.foreground": "{a}", "link.hover.foreground": "{a}", "link.active.foreground": "{a}",
            "list.background": "#22272e", "list.even.background": "#22272e", "list.head.background": "#2d333b",
            "list.hover.background": "#2d333b", "list.active.background": "{a}33", "list.active.border": "{a}",
            "table.background": "#22272e", "table.even.background": "#262c34", "table.head.background": "#22272e",
            "table.head.foreground": "#768390", "table.hover.background": "#2d333b",
            "table.active.background": "{a}33", "table.active.border": "{a}", "table.row.border": "#373e47b3",
            "sidebar.background": "#1c2128", "sidebar.foreground": "#adbac7", "sidebar.border": "#373e47",
            "sidebar.accent.background": "#2d333b", "sidebar.accent.foreground": "#cdd9e5",
            "sidebar.primary.background": "{a}", "sidebar.primary.foreground": "#ffffff",
            "title_bar.background": "#1c2128", "title_bar.border": "#373e47",
            "status_bar.background": "#1c2128", "status_bar.border": "#373e47",
            "tab_bar.background": "#1c2128", "tab.background": "#00000000", "tab.active.background": "#22272e",
            "tab.foreground": "#adbac7", "tab.active.foreground": "#cdd9e5",
            "scrollbar.background": "#22272e00", "scrollbar.thumb.background": "#545d68e6",
            "scrollbar.thumb.hover.background": "#636e7b",
            "slider.bar.background": "{a}", "slider.thumb.background": "#ffffff",
            "switch.background": "#444c56", "progress_bar.background": "{a}", "skeleton.background": "#2d333b",
            "group_box.background": "#2d333b", "group_box.foreground": "#cdd9e5",
            "drag_border": "{a}", "drop_target.background": "{a}26",
            "danger.background": "#e5534b", "danger.foreground": "#ffffff",
            "success.background": "#57ab5a", "success.foreground": "#ffffff",
            "warning.background": "#c69026", "warning.foreground": "#ffffff",
            "info.background": "#39c5cf", "info.foreground": "#ffffff",
            "window.border": "#373e47", "overlay": "#00000055"
        }}
    }}"##
    );
    serde_json::from_str(&json).expect("dimmed theme")
}

#[cfg(test)]
mod tests {
    use super::{Accent, dimmed};

    #[test]
    fn dimmed_theme_parses_with_every_accent() {
        for accent in Accent::ALL {
            let config = dimmed(accent);
            assert!(config.mode.is_dark());
        }
    }
}
