// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod assets;
mod drag_preview;
mod fs;
mod i18n;
mod icons;
mod settings;
mod shell;
mod shell_images;
mod table;
mod theme;
mod update;

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
        theme::apply(cx);
        icons::init(cx);
        app::bind_keys(cx);

        let view = settings::Settings::get(cx).view_mode;
        app::open_window(vec![app::Tab::new(start, view)], None, cx);

        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
}
