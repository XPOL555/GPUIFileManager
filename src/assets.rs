//! Asset source: gpui-kit's default icon bundle plus the few extra Lucide icons the
//! app uses, instead of the whole 1800-icon catalog.

use std::borrow::Cow;
use std::sync::Arc;

use gpui_kit::*;

gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        SquarePlus,
        AppWindow,
        Sheet,
        ListTree,
        List,
        Image,
        PanelLeft,
        PanelRight,
        Star,
        StarOff,
        ArrowLeftRight,
        SquareArrowOutUpRight,
        Move,
        FolderOpen,
        ExternalLink,
        Copy,
        Info,
        Eye,
        Ellipsis,
        RefreshCw,
        Menu,
    ]
);

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match ExtraIcons.load(path)? {
            Some(icon) => Ok(Some(icon)),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        Ok(paths)
    }
}

/// The app icon in full color (GPUI's `svg()` would draw it as a one-color mask).
pub fn logo() -> Arc<Image> {
    Arc::new(Image::from_bytes(ImageFormat::Svg, include_bytes!("../assets/icon.svg").to_vec()))
}
