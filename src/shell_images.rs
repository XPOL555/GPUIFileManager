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

    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
        DeleteObject, GetDIBits, GetObjectW, HBITMAP,
    };
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
    use windows::Win32::UI::Shell::{
        IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SHFILEINFOW, SHGFI_FLAGS,
        SHGFI_OVERLAYINDEX, SHGFI_SYSICONINDEX, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW, SHGetImageList,
        SHIL_EXTRALARGE, SHIL_JUMBO, SHIL_SMALL, SIIGBF_THUMBNAILONLY,
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
            flags |= SHGFI_OVERLAYINDEX;
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
        (ok != 0).then_some(info.iIcon)
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
    pub fn icon_bitmap(_index: i32, _size: IconSize) -> Option<Bitmap> {
        None
    }
    pub fn thumbnail(_path: &Path, _px: u32) -> Option<Bitmap> {
        None
    }
}
#[cfg(not(windows))]
pub use fallback::*;
