//! The benchmark's driver (`FM_BENCH`, see `crate::bench`). Each listing logs a `listed`
//! event when the frame showing it is painted; the first one is the startup. The steps
//! file then runs one line at a time:
//! `open <path>` (waits for its listing to be painted), `view details|list|m|l|xl`,
//! `scroll` (runs the view through to its end, a few rows a frame), `wait <ms>`,
//! `mem <label>`, `quit`.

use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use futures::channel::mpsc;
use serde_json::json;

use super::*;
use crate::bench;

#[derive(Default)]
pub(super) struct BenchMarks {
    /// When the listing in progress was asked for, and of which folder.
    started: Option<(Instant, PathBuf)>,
    /// Applied, waiting for its frame to be painted.
    listed: Option<serde_json::Value>,
    painted: Option<mpsc::UnboundedSender<()>>,
}

impl FileManager {
    /// Starts the driver in the first window, when `FM_BENCH` is set.
    pub(super) fn start_bench(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        static STARTED: AtomicBool = AtomicBool::new(false);
        if !bench::enabled() || STARTED.swap(true, Ordering::Relaxed) {
            return;
        }
        let steps = std::env::var_os("FM_BENCH_STEPS")
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        let (tx, mut painted) = mpsc::unbounded();
        self.bench.painted = Some(tx);
        cx.spawn_in(window, async move |this, cx| {
            painted.next().await;
            bench::event("startup", bench::memory());
            for line in steps.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
                let (step, arg) = line.split_once(' ').map_or((line, ""), |(s, a)| (s, a.trim()));
                match step {
                    "open" => {
                        let path = PathBuf::from(arg);
                        if this.update_in(cx, |fm, window, cx| fm.navigate(path, true, window, cx)).is_err() {
                            return;
                        }
                        painted.next().await;
                    }
                    "view" => {
                        let mode = match arg {
                            "list" => ViewMode::List,
                            "m" => ViewMode::MediumIcons,
                            "l" => ViewMode::LargeIcons,
                            "xl" => ViewMode::ExtraLargeIcons,
                            _ => ViewMode::Details,
                        };
                        this.update_in(cx, |fm, window, cx| fm.set_view_mode(mode, window, cx)).ok();
                    }
                    "scroll" => {
                        let mut item = 0;
                        loop {
                            let more = this.update(cx, |fm, cx| {
                                let items = fm.table.read(cx).delegate().len();
                                // Three rows of tiles, or of the table, a frame.
                                item += if fm.view().is_table() { 3 } else { 3 * fm.grid_columns.max(1) };
                                let row = item.min(items.saturating_sub(1));
                                if fm.view().is_table() {
                                    fm.scroll_to_row(row, cx);
                                } else {
                                    fm.scroll_grid_to(row);
                                }
                                cx.notify();
                                item < items
                            });
                            if !matches!(more, Ok(true)) {
                                break;
                            }
                            cx.background_executor().timer(Duration::from_millis(16)).await;
                        }
                        bench::event("scrolled", json!({ "items": item }));
                    }
                    "wait" => {
                        let ms = arg.parse().unwrap_or(1000);
                        cx.background_executor().timer(Duration::from_millis(ms)).await;
                    }
                    "mem" => {
                        let mut fields = bench::memory();
                        fields["label"] = arg.into();
                        bench::event("mem", fields);
                    }
                    "quit" => {
                        cx.update(|_, cx| cx.quit()).ok();
                        return;
                    }
                    _ => bench::event("unknown-step", json!({ "line": line })),
                }
            }
        })
        .detach();
    }

    /// A listing was asked for (`load`).
    pub(super) fn bench_loading(&mut self, path: &Path) {
        if bench::enabled() {
            self.bench.started = Some((Instant::now(), path.to_path_buf()));
        }
    }

    /// The listing is in the table: its frame is the next one painted.
    pub(super) fn bench_listed(&mut self, rows: usize) {
        if let Some((started, path)) = &self.bench.started {
            self.bench.listed = Some(json!({
                "path": path,
                "rows": rows,
                "list_ms": started.elapsed().as_secs_f64() * 1000.,
            }));
        }
    }

    /// Painted last in the window: logs the listing waiting for this frame.
    pub(super) fn bench_marker(&mut self) -> Option<impl IntoElement> {
        let mut fields = self.bench.listed.take()?;
        let (started, _) = self.bench.started.take()?;
        let painted = self.bench.painted.clone();
        Some(canvas(
            |_, _, _| {},
            move |_, _, _, _| {
                fields["paint_ms"] = (started.elapsed().as_secs_f64() * 1000.).into();
                if let (Some(fields), serde_json::Value::Object(memory)) = (fields.as_object_mut(), bench::memory()) {
                    fields.extend(memory);
                }
                bench::event("listed", fields);
                if let Some(painted) = painted {
                    painted.unbounded_send(()).ok();
                }
            },
        ))
    }
}
