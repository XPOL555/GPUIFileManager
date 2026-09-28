// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod assets;
mod fs;
mod i18n;
mod settings;
mod shell;
mod table;

use gpui_kit::component::{Theme, ThemeMode};

fn main() {
    shell::init_com();
    let start = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .or_else(|| shell::known_folders().into_iter().next().map(|(_, p)| p))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    gpui_kit::application().with_assets(assets::AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        cx.set_global(settings::Settings::load());
        i18n::init(cx);
        app::bind_keys(cx);
        Theme::change(ThemeMode::Dark, None, cx);

        app::open_window(vec![app::Tab::new(start)], None, cx);

        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
}
