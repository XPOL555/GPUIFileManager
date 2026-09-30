// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod assets;
mod context_menu;
mod dnd;
mod drag_preview;
mod file_ops;
mod fs;
mod hooks;
mod i18n;
mod icons;
mod recycle;
mod settings;
mod shell;
mod shell_images;
mod shell_menu;
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
        context_menu::bind_keys(cx);
        app::init(cx);

        app::open_window(vec![app::Tab::new(start, cx)], None, cx);

        // Explorer's menus, ready for the first right click (see `shell_menu::warm_up`).
        cx.spawn(async move |cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(1500)).await;
            if let Some(home) = shell::home() {
                shell_menu::warm_up(home.to_path_buf());
            }
        })
        .detach();

        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                // Quitting would cut a copy or a move short: let them finish first.
                cx.spawn(async move |cx| {
                    while file_ops::running() > 0 {
                        cx.background_executor().timer(std::time::Duration::from_millis(250)).await;
                    }
                    cx.update(|cx| cx.quit());
                })
                .detach();
            }
        })
        .detach();
    });
}
