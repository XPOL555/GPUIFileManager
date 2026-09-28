// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod fs;
mod i18n;
mod shell;
mod table;

use gpui_kit::component::{Theme, ThemeMode, TitleBar};
use gpui_kit::*;

fn main() {
    shell::init_com();
    let start = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .or_else(|| shell::known_folders().into_iter().next().map(|(_, p)| p))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        app::bind_keys(cx);
        i18n::set_language(i18n::Language::default(), cx);
        Theme::change(ThemeMode::Dark, None, cx);

        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        // The app draws its own title bar (`component::TitleBar`), with the settings menu on the left.
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("FileManager".into()), ..TitleBar::title_bar_options() }),
            ..TitleBar::window_options()
        };
        let (window, view) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| app::FileManager::new(start, window, cx))
        })
        .expect("failed to open window");
        let _ = window.update(cx, |_, window, cx| view.read(cx).focus_handle(cx).focus(window, cx));

        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
}
