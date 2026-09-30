//! Win32 side of icons and thumbnails: system image list icons and shell thumbnails
//! as premultiplied BGRA pixels. Everything here blocks, so it only runs on the
//! worker threads of `icons`.

use std::path::Path;

/// Pixels ready for GPUI's `RenderImage`: BGRA with straight alpha, rows top to bottom.
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl Bitmap {
    pub fn bytes(&self) -> usize {
        self.bgra.len()
    }
}

/// System image list sizes. The first three follow the system DPI, jumbo is 256 px.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconSize {
    /// Rows, sidebar, tabs (16 px at 100%).
    Small,
    /// Medium icon view (48 px).
    ExtraLarge,
    /// Large icon views (256 px).
    Jumbo,
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use super::*;
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::Foundation::{RECT, SIZE};
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
        DeleteObject, GetDIBits, GetObjectW, HBITMAP, HDC,
    };
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
    use windows::Win32::UI::Shell::{
        ILFree, IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SHFILEINFOW, SHGFI_FLAGS, SHGFI_ICON,
        SHGFI_OVERLAYINDEX, SHGFI_PIDL, SHGFI_SYSICONINDEX, SHGFI_USEFILEATTRIBUTES, SHGSI_SYSICONINDEX, SHGetFileInfoW,
        SHGetImageList, SHGetStockIconInfo, SHIL_EXTRALARGE, SHIL_JUMBO, SHIL_SMALL, SHParseDisplayName, SHSTOCKICONID,
        SHSTOCKICONINFO, SIIGBF_THUMBNAILONLY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};
    use windows::core::{Interface, PCWSTR};

    /// Worker threads call this once: shell icon and thumbnail handlers are apartment threaded.
    pub fn init_worker_thread() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
    }

    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }

    /// Index into the system image list, with the overlay (shortcut arrow) in the high
    /// byte when `overlay` is set. `from_attributes` skips the disk and answers from
    /// the name and `attrs` alone (same icon for every file of a type).
    pub fn icon_index(path: &Path, attrs: u32, from_attributes: bool, overlay: bool) -> Option<i32> {
        let w = wide(path);
        let mut info = SHFILEINFOW::default();
        let mut flags = SHGFI_SYSICONINDEX;
        if from_attributes {
            flags |= SHGFI_USEFILEATTRIBUTES;
        }
        if overlay {
            // The overlay index only comes with an icon handle.
            flags |= SHGFI_OVERLAYINDEX | SHGFI_ICON;
        }
        let ok = unsafe {
            SHGetFileInfoW(
                PCWSTR(w.as_ptr()),
                FILE_FLAGS_AND_ATTRIBUTES(attrs),
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_FLAGS(flags.0),
            )
        };
        if !info.hIcon.is_invalid() {
            unsafe {
                let _ = DestroyIcon(info.hIcon);
            }
        }
        (ok != 0).then_some(info.iIcon)
    }

    /// Index of a shell item that is not a file (`shell:` or `::{CLSID}` names), from
    /// the first of `names` the shell knows.
    pub fn shell_icon_index(names: &[&str]) -> Option<i32> {
        names.iter().find_map(|name| unsafe {
            let w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            let mut pidl = std::ptr::null_mut();
            SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None).ok()?;
            let mut info = SHFILEINFOW::default();
            let ok = SHGetFileInfoW(
                PCWSTR(pidl as *const u16),
                FILE_FLAGS_AND_ATTRIBUTES(0),
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_PIDL | SHGFI_SYSICONINDEX,
            );
            ILFree(Some(pidl));
            (ok != 0).then_some(info.iIcon)
        })
    }

    /// Index of one of the shell's stock icons (`SIID_*`).
    pub fn stock_icon_index(id: i32) -> Option<i32> {
        let mut info = SHSTOCKICONINFO { cbSize: std::mem::size_of::<SHSTOCKICONINFO>() as u32, ..Default::default() };
        unsafe { SHGetStockIconInfo(SHSTOCKICONID(id), SHGSI_SYSICONINDEX, &mut info) }.ok()?;
        Some(info.iSysImageIndex)
    }

    pub fn icon_bitmap(index: i32, size: IconSize) -> Option<Bitmap> {
        let list = match size {
            IconSize::Small => SHIL_SMALL,
            IconSize::ExtraLarge => SHIL_EXTRALARGE,
            IconSize::Jumbo => SHIL_JUMBO,
        };
        let bitmap = list_icon(index, list as i32)?;
        // Icons that only ship small images come out of the jumbo list as a 32 px
        // picture in the corner of a 256 px canvas: use the 48 px one instead.
        if size == IconSize::Jumbo && opaque_extent(&bitmap) <= 64 {
            return list_icon(index, SHIL_EXTRALARGE as i32);
        }
        Some(bitmap)
    }

    fn list_icon(index: i32, list: i32) -> Option<Bitmap> {
        unsafe {
            let images: IImageList = SHGetImageList(list).ok()?;
            let overlay = ((index as u32) >> 24) & 0xF;
            let flags = ILD_TRANSPARENT.0 | (overlay << 8);
            let icon = images.GetIcon(index & 0x00FF_FFFF, flags).ok()?;
            let bitmap = icon_pixels(icon);
            let _ = DestroyIcon(icon);
            bitmap
        }
    }

    /// Size of the smallest square, from the top-left corner, holding every visible pixel.
    fn opaque_extent(b: &Bitmap) -> u32 {
        let mut extent = 0;
        for (i, px) in b.bgra.chunks_exact(4).enumerate() {
            if px[3] != 0 {
                let (x, y) = (i as u32 % b.width, i as u32 / b.width);
                extent = extent.max(x + 1).max(y + 1);
            }
        }
        extent
    }

    unsafe fn icon_pixels(icon: HICON) -> Option<Bitmap> {
        unsafe {
            let mut info = ICONINFO::default();
            GetIconInfo(icon, &mut info).ok()?;
            let (color, mask) = (info.hbmColor, info.hbmMask);
            let result = if color.is_invalid() {
                None
            } else {
                // Icon color bitmaps carry straight alpha already.
                read_bitmap(color).map(|mut b| {
                    if b.bgra.chunks_exact(4).all(|px| px[3] == 0) {
                        // Old icon without alpha: the mask says what is transparent.
                        if let Some(m) = read_bitmap(mask) {
                            for (px, m) in b.bgra.chunks_exact_mut(4).zip(m.bgra.chunks_exact(4)) {
                                px[3] = if m[0] == 0 { 255 } else { 0 };
                            }
                        }
                    }
                    b
                })
            };
            if !color.is_invalid() {
                let _ = DeleteObject(color.into());
            }
            if !mask.is_invalid() {
                let _ = DeleteObject(mask.into());
            }
            result
        }
    }

    /// Any bitmap as 32 bpp top-down BGRA (alpha as stored).
    unsafe fn read_bitmap(bitmap: HBITMAP) -> Option<Bitmap> {
        unsafe {
            let mut bm = BITMAP::default();
            if GetObjectW(bitmap.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as _)) == 0 {
                return None;
            }
            let (width, height) = (bm.bmWidth, bm.bmHeight.abs());
            if width <= 0 || height <= 0 {
                return None;
            }
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bgra = vec![0u8; width as usize * height as usize * 4];
            let dc = CreateCompatibleDC(None);
            let lines = GetDIBits(dc, bitmap, 0, height as u32, Some(bgra.as_mut_ptr() as _), &mut info, DIB_RGB_COLORS);
            let _ = DeleteDC(dc);
            (lines == height).then_some(Bitmap { width: width as u32, height: height as u32, bgra })
        }
    }

    /// A menu item's bitmap (32 bpp premultiplied, or without alpha) as straight BGRA.
    /// `None` for the special values menus use in place of a bitmap.
    pub fn menu_bitmap(bitmap: HBITMAP) -> Option<Bitmap> {
        // HBMMENU_CALLBACK (-1) and the HBMMENU_* system glyphs (1 to 11). Real handles
        // are 32-bit values sign-extended: they may look negative.
        let value = bitmap.0 as isize;
        if value == -1 || (0..=11).contains(&value) {
            return None;
        }
        let mut b = unsafe { read_bitmap(bitmap)? };
        if b.bgra.chunks_exact(4).all(|px| px[3] == 0) {
            for px in b.bgra.chunks_exact_mut(4) {
                px[3] = 255;
            }
        } else {
            unpremultiply(&mut b.bgra);
        }
        Some(b)
    }

    /// Pixels something draws into a transparent `width` × `height` canvas (owner-drawn
    /// menu icons). Drawing without alpha leaves it at 0: those pixels count as opaque
    /// unless black.
    pub fn draw_to_bitmap(width: u32, height: u32, draw: impl FnOnce(HDC, RECT)) -> Option<Bitmap> {
        use windows::Win32::Graphics::Gdi::{CreateDIBSection, GdiFlush, SelectObject};
        if width == 0 || height == 0 || width > 256 || height > 256 {
            return None;
        }
        unsafe {
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    biHeight: -(height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let dc = CreateCompatibleDC(None);
            let mut bits = std::ptr::null_mut();
            let Ok(dib) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
                let _ = DeleteDC(dc);
                return None;
            };
            let old = SelectObject(dc, dib.into());
            draw(dc, RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 });
            let _ = GdiFlush();
            let len = (width * height * 4) as usize;
            let mut bgra = std::slice::from_raw_parts(bits as *const u8, len).to_vec();
            SelectObject(dc, old);
            let _ = DeleteObject(dib.into());
            let _ = DeleteDC(dc);
            if bgra.chunks_exact(4).all(|px| px[3] == 0) {
                for px in bgra.chunks_exact_mut(4) {
                    px[3] = if px[..3] == [0, 0, 0] { 0 } else { 255 };
                }
            } else {
                unpremultiply(&mut bgra);
            }
            // Nothing drawn at all.
            if bgra.chunks_exact(4).all(|px| px[3] == 0) {
                return None;
            }
            Some(Bitmap { width, height, bgra })
        }
    }

    fn unpremultiply(bgra: &mut [u8]) {
        for px in bgra.chunks_exact_mut(4) {
            let a = px[3] as u32;
            if a > 0 && a < 255 {
                for c in &mut px[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
    }

    /// The shell's thumbnail (images, videos, documents with a preview handler) no
    /// larger than `px`, or `None` when the item has no thumbnail, only an icon.
    pub fn thumbnail(path: &Path, px: u32) -> Option<Bitmap> {
        unsafe {
            let w = wide(path);
            let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None).ok()?;
            let factory: IShellItemImageFactory = item.cast().ok()?;
            let bitmap = factory.GetImage(SIZE { cx: px as i32, cy: px as i32 }, SIIGBF_THUMBNAILONLY).ok()?;
            let result = read_bitmap(bitmap).map(|mut b| {
                // The factory hands out premultiplied pixels; opaque formats come with alpha 0.
                if b.bgra.chunks_exact(4).all(|px| px[3] == 0) {
                    for px in b.bgra.chunks_exact_mut(4) {
                        px[3] = 255;
                    }
                } else {
                    unpremultiply(&mut b.bgra);
                }
                b
            });
            let _ = DeleteObject(bitmap.into());
            result
        }
    }
}

#[cfg(not(windows))]
mod fallback {
    use super::*;
    pub fn init_worker_thread() {}
    pub fn icon_index(_path: &Path, _attrs: u32, _from_attributes: bool, _overlay: bool) -> Option<i32> {
        None
    }
    pub fn shell_icon_index(_names: &[&str]) -> Option<i32> {
        None
    }
    pub fn stock_icon_index(_id: i32) -> Option<i32> {
        None
    }
    pub fn icon_bitmap(_index: i32, _size: IconSize) -> Option<Bitmap> {
        None
    }
    pub fn thumbnail(_path: &Path, _px: u32) -> Option<Bitmap> {
        None
    }
}
#[cfg(not(windows))]
pub use fallback::*;
