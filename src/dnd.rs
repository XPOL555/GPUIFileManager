//! Dragging files, with Explorer and between the app's windows. Every file drag is a
//! Windows (OLE) drag: the source hands over the shell's own data object, so Explorer
//! and other apps get every format they know, and each window has a drop target of its
//! own in place of GPUI's, which always answers "copy". What lies under the cursor
//! comes from the drop zones the views record while painting.
//!
//! The drop target runs inside Windows' drag loop, on the UI thread but outside any
//! GPUI update: it only reads the zones and sends `hooks::HookEvent`s to the app.

use std::cell::RefCell;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use gpui_kit::*;

/// What a drop zone is, so the view can highlight it while a drag hovers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpotKey {
    /// The background of the file view: the listed folder.
    View,
    /// A folder in the file view.
    Item(PathBuf),
    /// A tab: its folder.
    Tab(usize),
    /// A favorite, a drive or the Recycle Bin.
    Sidebar(PathBuf),
    /// A breadcrumb segment.
    Crumb(usize),
    /// The favorites heading: folders dropped there become favorites.
    Favorites,
}

/// What a drop zone does with files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Copies or moves them into a folder.
    Folder(PathBuf),
    /// Adds folders to the favorites.
    Favorites,
    /// Deletes them to the Recycle Bin.
    RecycleBin,
}

impl From<PathBuf> for Target {
    fn from(path: PathBuf) -> Self {
        if crate::fs::is_recycle_bin(&path) { Target::RecycleBin } else { Target::Folder(path) }
    }
}

struct DropZone {
    bounds: Bounds<Pixels>,
    key: SpotKey,
    target: Target,
    /// Zones inside other zones (a folder row inside the view) have a higher one.
    priority: u8,
}

/// Where files can be dropped in one window, as painted in the last frame.
#[derive(Default)]
pub struct DropZones {
    zones: Vec<DropZone>,
}

pub type SharedZones = Rc<RefCell<DropZones>>;

impl DropZones {
    /// Called at the start of every frame; painting records the zones again.
    pub fn clear(&mut self) {
        self.zones.clear();
    }

    fn add(&mut self, bounds: Bounds<Pixels>, key: SpotKey, target: Target, priority: u8) {
        if bounds.size.width > px(0.) && bounds.size.height > px(0.) {
            self.zones.push(DropZone { bounds, key, target, priority });
        }
    }

    /// The zone under `point` (window coordinates): the innermost one.
    fn at(&self, point: Point<Pixels>) -> Option<(&SpotKey, &Target)> {
        self.zones
            .iter()
            .filter(|z| z.bounds.contains(&point))
            .max_by_key(|z| z.priority)
            .map(|z| (&z.key, &z.target))
    }
}

/// An `on_prepaint` callback recording a drop zone (a folder, usually), clipped to the
/// visible part of the element (rows scrolled half out of a list).
pub fn zone(
    zones: &SharedZones,
    key: SpotKey,
    target: impl Into<Target>,
    priority: u8,
) -> impl FnOnce(Bounds<Pixels>, &mut Window, &mut App) + 'static {
    let (zones, target) = (zones.clone(), target.into());
    move |bounds, window, _| {
        let visible = bounds.intersect(&window.content_mask().bounds);
        if let Ok(mut zones) = zones.try_borrow_mut() {
            zones.add(visible, key, target, priority);
        }
    }
}

/// What a drop does with the files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropKind {
    Copy,
    Move,
    /// To the Recycle Bin.
    Recycle,
    /// The folders become favorites.
    Favorite,
}

/// Whether a path is inside a Recycle Bin (items listed from it).
fn in_bin(path: &Path) -> bool {
    path_key(path).contains("\\$recycle.bin\\")
}

/// A drop on a zone that is not a folder.
fn special_drop(target: &Target, sources: &[PathBuf], has_folders: bool) -> Option<DropKind> {
    match target {
        Target::Folder(_) => None,
        Target::Favorites => has_folders.then_some(DropKind::Favorite),
        Target::RecycleBin => (!sources.is_empty() && !sources.iter().any(|p| in_bin(p))).then_some(DropKind::Recycle),
    }
}

/// Explorer's rules: Ctrl copies, Shift moves, otherwise files move within a drive and
/// are copied to another one. `None` when the drop would do nothing or can't be done:
/// a folder into itself or below itself, or a move into the folder the files are in.
pub fn drop_kind(
    sources: &[PathBuf],
    dest: &Path,
    ctrl: bool,
    shift: bool,
    can_copy: bool,
    can_move: bool,
) -> Option<DropKind> {
    let first = sources.first()?;
    let dest_key = path_key(dest);
    let inside = |source: &PathBuf| {
        let source = path_key(source);
        dest_key == source || dest_key.starts_with(&format!("{source}\\"))
    };
    if sources.iter().any(inside) {
        return None;
    }
    let wanted = match (ctrl, shift) {
        (true, false) => DropKind::Copy,
        (false, true) => DropKind::Move,
        _ if same_volume(first, dest) => DropKind::Move,
        _ => DropKind::Copy,
    };
    let all_here = sources.iter().all(|s| s.parent().is_some_and(|p| path_key(p) == dest_key));
    let kind = match wanted {
        DropKind::Move if can_move => DropKind::Move,
        _ if can_copy => DropKind::Copy,
        _ if can_move => DropKind::Move,
        _ => return None,
    };
    // Moving files where they already are does nothing; copying them there makes copies.
    (kind == DropKind::Copy || !all_here).then_some(kind)
}

/// Windows paths compare without case and without a trailing separator.
fn path_key(path: &Path) -> String {
    let key = path.to_string_lossy().replace('/', "\\").to_lowercase();
    key.trim_end_matches('\\').to_string()
}

/// Same drive letter, or same `\\server\share`.
fn same_volume(a: &Path, b: &Path) -> bool {
    let root = |p: &Path| match p.components().next() {
        Some(Component::Prefix(prefix)) => Some(prefix.as_os_str().to_string_lossy().to_lowercase()),
        _ => None,
    };
    root(a).is_some() && root(a) == root(b)
}

/// The text Windows shows under the dragged image, in the current language.
#[derive(Clone, Copy)]
pub struct DropLabels {
    /// "Move to %1".
    pub move_to: &'static str,
    pub copy_to: &'static str,
    /// "Add to %1", with `favorites` for %1.
    pub add_to: &'static str,
    pub favorites: &'static str,
    pub recycle_bin: &'static str,
}

thread_local! {
    /// Set by `i18n`.
    static LABELS: RefCell<DropLabels> = const {
        RefCell::new(DropLabels {
            move_to: "Move to %1",
            copy_to: "Copy to %1",
            add_to: "Add to %1",
            favorites: "Favorites",
            recycle_bin: "Recycle Bin",
        })
    };
}

pub fn set_labels(labels: DropLabels) {
    LABELS.set(labels);
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use super::*;
    use std::cell::RefCell;

    use windows::Win32::Foundation::{GlobalFree, HWND, POINT, POINTL};
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, CoCreateInstance, DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, STGMEDIUM_0,
        TYMED_HGLOBAL,
    };
    use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::{
        DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE, IDropTarget,
        IDropTarget_Impl, RegisterDragDrop, RevokeDragDrop,
    };
    use windows::Win32::System::SystemServices::{MK_CONTROL, MK_SHIFT, MODIFIERKEYS_FLAGS};
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_LBUTTON};
    use windows::Win32::UI::Shell::{
        BHID_DataObject, CLSID_DragDropHelper, DROPDESCRIPTION, DROPIMAGE_COPY, DROPIMAGE_INVALID, DROPIMAGE_LINK,
        DROPIMAGE_MOVE, DROPIMAGETYPE, DSH_ALLOWDROPDESCRIPTIONTEXT, IDragSourceHelper2, IDropTargetHelper, SHDoDragDrop,
    };
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::core::{Ref, implement, w};

    use crate::file_ops;
    use crate::hooks::{self, HookEvent};

    /// Replaces GPUI's drop target of `hwnd` with one that knows the window's drop zones.
    pub fn register_drop_target(hwnd: isize, zones: SharedZones) {
        let hwnd = HWND(hwnd as _);
        let helper: Option<IDropTargetHelper> =
            unsafe { CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER) }.ok();
        let target: IDropTarget = DropTarget { hwnd, zones, helper, drag: RefCell::new(None) }.into();
        unsafe {
            let _ = RevokeDragDrop(hwnd);
            if let Err(err) = RegisterDragDrop(hwnd, &target) {
                eprintln!("register drop target: {err}");
            }
        }
    }

    /// Drags `paths` until the mouse button is released, over any window, and returns
    /// whether they were dropped somewhere. It runs Windows' modal drag loop, which also
    /// drives the app's own drop targets: never call it while a GPUI borrow is held, see
    /// `hooks::start_file_drag`. Nothing is deleted here: whoever takes a move moves the
    /// files.
    pub fn drag_files(paths: &[PathBuf], hwnd: isize) -> bool {
        // The button was released before the drag could start: don't drop anywhere.
        if unsafe { GetKeyState(VK_LBUTTON.0 as i32) } >= 0 {
            return false;
        }
        let Ok(items) = file_ops::shell_items(paths) else { return false };
        let Ok(data) = (unsafe { items.BindToHandler::<_, IDataObject>(None, &BHID_DataObject) }) else {
            return false;
        };
        unsafe {
            // The shell's drag image of the items, with room for the "Move to …" text.
            if let Ok(helper) = CoCreateInstance::<_, IDragSourceHelper2>(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER) {
                let _ = helper.SetFlags(DSH_ALLOWDROPDESCRIPTIONTEXT.0 as u32);
                let _ = helper.InitializeFromWindow(None, None, &data);
            }
            let allowed = DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK;
            matches!(SHDoDragDrop(Some(HWND(hwnd as _)), &data, None, allowed), Ok(effect) if effect != DROPEFFECT_NONE)
        }
    }

    #[implement(IDropTarget)]
    struct DropTarget {
        hwnd: HWND,
        zones: SharedZones,
        helper: Option<IDropTargetHelper>,
        drag: RefCell<Option<Drag>>,
    }

    /// The drag currently over the window.
    struct Drag {
        data: IDataObject,
        paths: Vec<PathBuf>,
        /// Some of the files are folders (they can become favorites).
        has_folders: bool,
        spot: Option<SpotKey>,
        /// Where a drop would go and what it would do.
        drop: Option<(Target, DropKind)>,
        /// The text last put under the drag image.
        described: Option<(Target, DropKind)>,
    }

    impl DropTarget {
        /// Window coordinates of a screen point.
        fn to_window(&self, pt: &POINTL) -> Point<Pixels> {
            let mut p = POINT { x: pt.x, y: pt.y };
            unsafe {
                let _ = ScreenToClient(self.hwnd, &mut p);
                let scale = GetDpiForWindow(self.hwnd).max(1) as f32 / 96.;
                point(px(p.x as f32 / scale), px(p.y as f32 / scale))
            }
        }

        /// Finds the zone under the cursor and what a drop there would do; returns the
        /// effect Windows shows. `allowed` are the effects the source offers.
        fn track(&self, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, allowed: DROPEFFECT) -> DROPEFFECT {
            let mut drag = self.drag.borrow_mut();
            let Some(drag) = drag.as_mut() else { return DROPEFFECT_NONE };
            let position = self.to_window(pt);
            let (spot, drop) = match self
                .zones
                .try_borrow()
                .ok()
                .as_deref()
                .and_then(|z| z.at(position).map(|(key, target)| (key.clone(), target.clone())))
            {
                Some((key, Target::Folder(folder))) => {
                    let kind = drop_kind(
                        &drag.paths,
                        &folder,
                        keys.contains(MK_CONTROL),
                        keys.contains(MK_SHIFT),
                        allowed.contains(DROPEFFECT_COPY),
                        allowed.contains(DROPEFFECT_MOVE),
                    );
                    (Some(key), kind.map(|kind| (Target::Folder(folder), kind)))
                }
                Some((key, target)) => {
                    let kind = special_drop(&target, &drag.paths, drag.has_folders);
                    (Some(key), kind.map(|kind| (target, kind)))
                }
                None => (None, None),
            };
            // Highlight only the zones that would take the drop.
            let spot = spot.filter(|_| drop.is_some());
            if spot != drag.spot {
                drag.spot = spot.clone();
                hooks::send(self.hwnd.0 as isize, HookEvent::DropHover(spot));
            }
            if drop != drag.described {
                describe(&drag.data, drop.as_ref());
                drag.described = drop.clone();
            }
            drag.drop = drop;
            match drag.drop {
                Some((_, DropKind::Copy)) => DROPEFFECT_COPY,
                Some((_, DropKind::Move | DropKind::Recycle)) => DROPEFFECT_MOVE,
                Some((_, DropKind::Favorite)) => DROPEFFECT_LINK,
                None => DROPEFFECT_NONE,
            }
        }

        fn end(&self) {
            let hwnd = self.hwnd.0 as isize;
            if let Some(drag) = self.drag.borrow_mut().take() {
                if drag.spot.is_some() {
                    hooks::send(hwnd, HookEvent::DropHover(None));
                }
                if drag.has_folders {
                    hooks::send(hwnd, HookEvent::DraggingFolders(false));
                }
            }
        }
    }

    #[allow(non_snake_case)]
    impl IDropTarget_Impl for DropTarget_Impl {
        fn DragEnter(
            &self,
            pdataobj: Ref<IDataObject>,
            grfkeystate: MODIFIERKEYS_FLAGS,
            pt: &POINTL,
            pdweffect: *mut DROPEFFECT,
        ) -> windows::core::Result<()> {
            let data = pdataobj.ok()?.clone();
            let paths = file_ops::data_object_paths(&data);
            let has_folders = paths.iter().take(64).any(|p| p.is_dir());
            if has_folders {
                // The favorites heading shows where folders can go.
                hooks::send(self.hwnd.0 as isize, HookEvent::DraggingFolders(true));
            }
            *self.drag.borrow_mut() =
                Some(Drag { data: data.clone(), paths, has_folders, spot: None, drop: None, described: None });
            unsafe {
                let effect = self.track(grfkeystate, pt, *pdweffect);
                *pdweffect = effect;
                if let Some(helper) = &self.helper {
                    let _ = helper.DragEnter(self.hwnd, &data, &POINT { x: pt.x, y: pt.y }, effect);
                }
            }
            Ok(())
        }

        fn DragOver(&self, grfkeystate: MODIFIERKEYS_FLAGS, pt: &POINTL, pdweffect: *mut DROPEFFECT) -> windows::core::Result<()> {
            unsafe {
                let effect = self.track(grfkeystate, pt, *pdweffect);
                *pdweffect = effect;
                if let Some(helper) = &self.helper {
                    let _ = helper.DragOver(&POINT { x: pt.x, y: pt.y }, effect);
                }
            }
            Ok(())
        }

        fn DragLeave(&self) -> windows::core::Result<()> {
            if let Some(helper) = &self.helper {
                unsafe {
                    let _ = helper.DragLeave();
                }
            }
            // Other windows may not describe drops: don't leave "Move to …" behind.
            if let Some(drag) = self.drag.borrow().as_ref().filter(|d| d.described.is_some()) {
                describe(&drag.data, None);
            }
            self.end();
            Ok(())
        }

        fn Drop(
            &self,
            pdataobj: Ref<IDataObject>,
            grfkeystate: MODIFIERKEYS_FLAGS,
            pt: &POINTL,
            pdweffect: *mut DROPEFFECT,
        ) -> windows::core::Result<()> {
            let data = pdataobj.ok()?.clone();
            unsafe {
                let effect = self.track(grfkeystate, pt, *pdweffect);
                if let Some(helper) = &self.helper {
                    let _ = helper.Drop(&data, &POINT { x: pt.x, y: pt.y }, effect);
                }
                let dropped = self.drag.borrow().as_ref().and_then(|d| d.drop.clone().map(|drop| (d.paths.clone(), drop)));
                // The app moves the files itself: answering "move" would let the source
                // delete them, maybe before they are copied. Explorer calls this an
                // optimized move and expects "none" back.
                *pdweffect = match &dropped {
                    Some((_, (_, DropKind::Copy))) => DROPEFFECT_COPY,
                    Some((_, (_, DropKind::Favorite))) => DROPEFFECT_LINK,
                    _ => DROPEFFECT_NONE,
                };
                self.end();
                if let Some((paths, (target, kind))) = dropped {
                    hooks::send(self.hwnd.0 as isize, HookEvent::DropFiles { paths, target, kind });
                }
            }
            Ok(())
        }
    }

    /// The text under the drag image ("Move to Documents"); `None` restores the default.
    fn describe(data: &IDataObject, drop: Option<&(Target, DropKind)>) {
        let mut desc = DROPDESCRIPTION { r#type: DROPIMAGE_INVALID, ..Default::default() };
        if let Some((target, kind)) = drop {
            let labels = LABELS.with_borrow(|labels| *labels);
            let (message, image): (&str, DROPIMAGETYPE) = match kind {
                DropKind::Move | DropKind::Recycle => (labels.move_to, DROPIMAGE_MOVE),
                DropKind::Copy => (labels.copy_to, DROPIMAGE_COPY),
                DropKind::Favorite => (labels.add_to, DROPIMAGE_LINK),
            };
            let name = match target {
                Target::Folder(folder) => folder
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| folder.to_string_lossy().into_owned()),
                Target::Favorites => labels.favorites.to_string(),
                Target::RecycleBin => labels.recycle_bin.to_string(),
            };
            // A packed struct: its fields can't be borrowed, only assigned.
            desc.r#type = image;
            desc.szMessage = wide_array(message);
            desc.szInsert = wide_array(&name);
        }
        unsafe {
            let format = FORMATETC {
                cfFormat: RegisterClipboardFormatW(w!("DropDescription")) as u16,
                ptd: std::ptr::null_mut(),
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
            };
            let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, size_of::<DROPDESCRIPTION>()) else { return };
            let ptr = GlobalLock(memory) as *mut DROPDESCRIPTION;
            if ptr.is_null() {
                let _ = GlobalFree(Some(memory));
                return;
            }
            ptr.write(desc);
            let _ = GlobalUnlock(memory);
            let medium = STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 { hGlobal: memory },
                pUnkForRelease: std::mem::ManuallyDrop::new(None),
            };
            // With `release` the data object owns the memory from now on.
            if data.SetData(&format, &medium, true).is_err() {
                let _ = GlobalFree(Some(memory));
            }
        }
    }

    /// `s` as a fixed, null-terminated UTF-16 buffer, cut if too long.
    fn wide_array(s: &str) -> [u16; 260] {
        let mut buffer = [0u16; 260];
        for (slot, c) in buffer.iter_mut().zip(s.encode_utf16().take(259)) {
            *slot = c;
        }
        buffer
    }
}

#[cfg(not(windows))]
mod fallback {
    use super::*;
    pub fn register_drop_target(_hwnd: isize, _zones: SharedZones) {}
    pub fn drag_files(_paths: &[PathBuf], _hwnd: isize) -> bool {
        false
    }
}
#[cfg(not(windows))]
pub use fallback::*;

#[cfg(test)]
mod tests {
    use super::{DropKind, drop_kind};
    use std::path::PathBuf;

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn drops_follow_explorer_rules() {
        let files = paths(&[r"C:\a\x.txt", r"C:\a\y.txt"]);
        let any = |dest: &str, ctrl, shift| drop_kind(&files, dest.as_ref(), ctrl, shift, true, true);
        // Same drive moves, another drive copies; Ctrl and Shift force either.
        assert_eq!(any(r"C:\b", false, false), Some(DropKind::Move));
        assert_eq!(any(r"D:\b", false, false), Some(DropKind::Copy));
        assert_eq!(any(r"C:\b", true, false), Some(DropKind::Copy));
        assert_eq!(any(r"D:\b", false, true), Some(DropKind::Move));
        // Moving into the folder the files are in does nothing; copying there duplicates.
        assert_eq!(any(r"c:\A\", false, false), None);
        assert_eq!(any(r"C:\a", true, false), Some(DropKind::Copy));
        // The source only allows copying.
        assert_eq!(drop_kind(&files, r"C:\b".as_ref(), false, false, true, false), Some(DropKind::Copy));
        assert_eq!(drop_kind(&files, r"C:\b".as_ref(), false, false, false, false), None);
    }

    #[test]
    fn special_zones_take_what_they_can() {
        use super::{Target, special_drop};
        let files = paths(&[r"C:\a\x.txt"]);
        assert_eq!(special_drop(&Target::RecycleBin, &files, false), Some(DropKind::Recycle));
        // Items of the bin itself, or nothing at all.
        assert_eq!(special_drop(&Target::RecycleBin, &paths(&[r"C:\$Recycle.Bin\S-1\$R1.txt"]), false), None);
        assert_eq!(special_drop(&Target::RecycleBin, &[], false), None);
        // Only folders become favorites.
        assert_eq!(special_drop(&Target::Favorites, &files, false), None);
        assert_eq!(special_drop(&Target::Favorites, &files, true), Some(DropKind::Favorite));
        assert_eq!(Target::from(PathBuf::from(crate::fs::RECYCLE_BIN)), Target::RecycleBin);
    }

    #[test]
    fn a_folder_never_drops_into_itself() {
        let folder = paths(&[r"C:\a\sub"]);
        assert_eq!(drop_kind(&folder, r"C:\a\sub".as_ref(), false, false, true, true), None);
        assert_eq!(drop_kind(&folder, r"C:\A\Sub\deeper".as_ref(), true, false, true, true), None);
        // A sibling whose name starts the same is fine.
        assert_eq!(drop_kind(&folder, r"C:\a\sub2".as_ref(), false, false, true, true), Some(DropKind::Move));
        assert_eq!(drop_kind(&[], r"C:\a".as_ref(), false, false, true, true), None);
    }
}
