//! Renders `assets/icon.svg` into `assets/app.ico` (16–256 px, embedded by `build.rs`)
//! and `assets/icon.png` (256 px, for the README). Run after editing the SVG:
//!
//! ```sh
//! cargo run --manifest-path tools/icon/Cargo.toml
//! ```

use std::path::Path;

use resvg::{tiny_skia, usvg};

const SIZES: [u32; 9] = [16, 20, 24, 32, 40, 48, 64, 128, 256];

fn main() {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let svg = std::fs::read(assets.join("icon.svg")).expect("read icon.svg");
    let tree = usvg::Tree::from_data(&svg, &usvg::Options::default()).expect("parse icon.svg");

    let mut icon = ico::IconDir::new(ico::ResourceType::Icon);
    for size in SIZES {
        let mut pixmap = tiny_skia::Pixmap::new(size, size).unwrap();
        let scale = size as f32 / tree.size().width();
        resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
        if size == 256 {
            pixmap.save_png(assets.join("icon.png")).expect("write icon.png");
        }
        // tiny-skia keeps premultiplied alpha; ICO wants straight alpha.
        let rgba = pixmap
            .pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect();
        let image = ico::IconImage::from_rgba_data(size, size, rgba);
        icon.add_entry(ico::IconDirEntry::encode(&image).expect("encode icon"));
    }
    let file = std::fs::File::create(assets.join("app.ico")).expect("create app.ico");
    icon.write(file).expect("write app.ico");
    println!("wrote assets/app.ico and assets/icon.png");
}
