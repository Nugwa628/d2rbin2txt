//! d2rbin2txt: a small window. Drop D2R excel .bin files (or folders of them) on it, or on the .exe, and the .txt
//! files are written next to them. Plain Win32 (user32 / shell32 / comdlg32), no crates.
//! `d2rbin2txt --cli <files or folders> [--out <folder>]` converts without a window and prints the results.
#![windows_subsystem = "windows"]

mod d2rbin;

use std::ffi::{c_void, OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicIsize, Ordering};

type Hwnd = *mut c_void;
type WndProc = unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize;

#[repr(C)]
struct WndClassW {
    style: u32,
    proc_: WndProc,
    cls_extra: i32,
    wnd_extra: i32,
    instance: *mut c_void,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: *mut c_void,
    menu_name: *const u16,
    class_name: *const u16,
}

#[repr(C)]
struct Msg {
    hwnd: Hwnd,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: [i32; 2],
}

#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct OpenFileNameW {
    size: u32,
    owner: Hwnd,
    instance: *mut c_void,
    filter: *const u16,
    custom_filter: *mut u16,
    max_cust_filter: u32,
    filter_index: u32,
    file: *mut u16,
    max_file: u32,
    file_title: *mut u16,
    max_file_title: u32,
    initial_dir: *const u16,
    title: *const u16,
    flags: u32,
    file_offset: u16,
    file_extension: u16,
    def_ext: *const u16,
    cust_data: isize,
    hook: *mut c_void,
    template_name: *const u16,
    reserved_ptr: *mut c_void,
    reserved: u32,
    flags_ex: u32,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassW(c: *const WndClassW) -> u16;
    fn CreateWindowExW(ex: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32,
                       parent: Hwnd, menu: *mut c_void, inst: *mut c_void, param: *mut c_void) -> Hwnd;
    fn DefWindowProcW(h: Hwnd, m: u32, w: usize, l: isize) -> isize;
    fn GetMessageW(m: *mut Msg, h: Hwnd, a: u32, b: u32) -> i32;
    fn TranslateMessage(m: *const Msg) -> i32;
    fn DispatchMessageW(m: *const Msg) -> isize;
    fn PostQuitMessage(code: i32);
    fn SendMessageW(h: Hwnd, m: u32, w: usize, l: isize) -> isize;
    fn MoveWindow(h: Hwnd, x: i32, y: i32, w: i32, hh: i32, repaint: i32) -> i32;
    fn GetClientRect(h: Hwnd, r: *mut Rect) -> i32;
    fn LoadCursorW(inst: *mut c_void, name: *const u16) -> *mut c_void;
    fn LoadIconW(inst: *mut c_void, name: *const u16) -> *mut c_void;
    fn GetWindowTextLengthW(h: Hwnd) -> i32;
    fn SetProcessDPIAware() -> i32;
    fn GetDpiForWindow(h: Hwnd) -> u32;
}

#[link(name = "shell32")]
extern "system" {
    fn DragAcceptFiles(h: Hwnd, accept: i32);
    fn DragQueryFileW(drop: *mut c_void, i: u32, buf: *mut u16, n: u32) -> u32;
    fn DragFinish(drop: *mut c_void);
}

#[link(name = "comdlg32")]
extern "system" {
    fn GetOpenFileNameW(o: *mut OpenFileNameW) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateFontW(h: i32, w: i32, esc: i32, ori: i32, weight: i32, italic: u32, underline: u32, strike: u32,
                   charset: u32, out_prec: u32, clip: u32, quality: u32, pitch: u32, face: *const u16) -> *mut c_void;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
}

const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_VSCROLL: u32 = 0x0020_0000;
const WS_HSCROLL: u32 = 0x0010_0000;
const WS_EX_CLIENTEDGE: u32 = 0x200;
const WS_EX_ACCEPTFILES: u32 = 0x10;
const ES_MULTILINE: u32 = 0x4;
const ES_AUTOVSCROLL: u32 = 0x40;
const ES_AUTOHSCROLL: u32 = 0x80;
const ES_READONLY: u32 = 0x800;
const SS_CENTER: u32 = 0x1;
const BS_PUSHBUTTON: u32 = 0;
const WM_DESTROY: u32 = 0x2;
const WM_SIZE: u32 = 0x5;
const WM_SETFONT: u32 = 0x30;
const WM_COMMAND: u32 = 0x111;
const WM_DROPFILES: u32 = 0x233;
const EM_SETSEL: u32 = 0xB1;
const EM_REPLACESEL: u32 = 0xC2;
const ID_BROWSE: usize = 1;
const OFN_FLAGS: u32 = 0x0020_0000 | 0x0008_0000 | 0x200 | 0x1000; // explorer, long names, multi-select, file must exist

static EDIT: AtomicIsize = AtomicIsize::new(0);
static LABEL: AtomicIsize = AtomicIsize::new(0);
static BUTTON: AtomicIsize = AtomicIsize::new(0);

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain([0]).collect()
}

fn hwnd(a: &AtomicIsize) -> Hwnd {
    a.load(Ordering::Relaxed) as Hwnd
}

fn log(text: &str) {
    let edit = hwnd(&EDIT);
    let w = wide(&text.replace('\n', "\r\n"));
    unsafe {
        let n = GetWindowTextLengthW(edit) as usize;
        SendMessageW(edit, EM_SETSEL, n, n as isize);
        SendMessageW(edit, EM_REPLACESEL, 0, w.as_ptr() as isize);
    }
}

fn convert_all(paths: &[PathBuf], out: Option<&Path>, mut say: impl FnMut(&str)) -> usize {
    let mut bad = 0;
    for p in paths {
        let files = d2rbin::expand(p);
        if files.is_empty() {
            say(&format!("{}: no .bin files in this folder\n", p.display()));
        }
        for f in files {
            match d2rbin::convert(&f, out) {
                Ok(lines) => lines.iter().for_each(|l| say(&format!("{}\n", l))),
                Err(e) => {
                    bad += 1;
                    say(&format!("{}: {}\n", f.file_name().unwrap_or_default().to_string_lossy(), e));
                }
            }
        }
    }
    bad
}

fn convert_and_log(paths: &[PathBuf]) {
    if paths.is_empty() {
        return;
    }
    let bad = convert_all(paths, None, log);
    let folder = paths[0].parent().map(|p| p.display().to_string()).unwrap_or_default();
    log(&if bad == 0 { format!("Done. The .txt files are next to the .bin files ({}).\n\n", folder) }
         else { format!("Done, {} file(s) could not be read.\n\n", bad) });
}

fn browse(owner: Hwnd) -> Vec<PathBuf> {
    let filter: Vec<u16> = "D2R excel .bin\0*.bin\0All files\0*.*\0\0".encode_utf16().collect();
    let title = wide("Choose D2R .bin files");
    let mut buf = vec![0u16; 64 * 1024];
    let mut o = OpenFileNameW {
        size: std::mem::size_of::<OpenFileNameW>() as u32, owner, instance: null_mut(), filter: filter.as_ptr(),
        custom_filter: null_mut(), max_cust_filter: 0, filter_index: 1, file: buf.as_mut_ptr(), max_file: buf.len() as u32,
        file_title: null_mut(), max_file_title: 0, initial_dir: null(), title: title.as_ptr(), flags: OFN_FLAGS,
        file_offset: 0, file_extension: 0, def_ext: null(), cust_data: 0, hook: null_mut(), template_name: null(),
        reserved_ptr: null_mut(), reserved: 0, flags_ex: 0,
    };
    if unsafe { GetOpenFileNameW(&mut o) } == 0 {
        return vec![];
    }
    // one file: the full path; several: the folder, then each name, separated by 0s, ending in two 0s
    let parts: Vec<OsString> = buf.split(|&c| c == 0).take_while(|s| !s.is_empty()).map(OsString::from_wide).collect();
    match parts.len() {
        0 => vec![],
        1 => vec![PathBuf::from(&parts[0])],
        _ => parts[1..].iter().map(|n| PathBuf::from(&parts[0]).join(n)).collect(),
    }
}

fn layout(h: Hwnd) {
    let mut r = Rect { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe {
        GetClientRect(h, &mut r);
        let s = GetDpiForWindow(h).max(96) as i32;
        let px = |v: i32| v * s / 96;
        let (w, hh) = (r.right, r.bottom);
        MoveWindow(hwnd(&LABEL), px(10), px(12), w - px(140), px(40), 1);
        MoveWindow(hwnd(&BUTTON), w - px(120), px(14), px(110), px(30), 1);
        MoveWindow(hwnd(&EDIT), px(10), px(60), w - px(20), hh - px(70), 1);
    }
}

unsafe extern "system" fn wnd_proc(h: Hwnd, m: u32, w: usize, l: isize) -> isize {
    match m {
        WM_DROPFILES => {
            let drop = w as *mut c_void;
            let n = DragQueryFileW(drop, u32::MAX, null_mut(), 0);
            let mut paths = Vec::new();
            for i in 0..n {
                let len = DragQueryFileW(drop, i, null_mut(), 0) as usize;
                let mut buf = vec![0u16; len + 1];
                DragQueryFileW(drop, i, buf.as_mut_ptr(), buf.len() as u32);
                paths.push(PathBuf::from(OsString::from_wide(&buf[..len])));
            }
            DragFinish(drop);
            convert_and_log(&paths);
            0
        }
        WM_COMMAND if w & 0xFFFF == ID_BROWSE => {
            convert_and_log(&browse(h));
            0
        }
        WM_SIZE => {
            layout(h);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(h, m, w, l),
    }
}

fn cli(args: &[OsString]) -> i32 {
    let mut out = None;
    let mut paths = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--out" {
            out = it.next().map(PathBuf::from);
        } else {
            paths.push(PathBuf::from(a));
        }
    }
    if paths.is_empty() {
        println!("usage: d2rbin2txt --cli <file.bin | folder> [...] [--out <folder>]");
        return 1;
    }
    let bad = convert_all(&paths, out.as_deref(), |s| print!("{}", s));
    if bad == 0 { 0 } else { 2 }
}

fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.first().map_or(false, |a| a == "--cli") {
        std::process::exit(cli(&args[1..]));
    }
    unsafe {
        SetProcessDPIAware();
        let inst = GetModuleHandleW(null());
        let class = wide("D2RBin2Txt");
        let wc = WndClassW {
            style: 0, proc_: wnd_proc, cls_extra: 0, wnd_extra: 0, instance: inst,
            icon: LoadIconW(null_mut(), 32512 as *const u16), cursor: LoadCursorW(null_mut(), 32512 as *const u16),
            background: 16 as *mut c_void, // COLOR_BTNFACE + 1
            menu_name: null(), class_name: class.as_ptr(),
        };
        RegisterClassW(&wc);
        let main = CreateWindowExW(WS_EX_ACCEPTFILES, class.as_ptr(), wide("D2R .bin to .txt").as_ptr(),
                                   WS_OVERLAPPEDWINDOW | WS_VISIBLE, 200, 200, 760, 460, null_mut(), null_mut(), inst, null_mut());
        let dpi = GetDpiForWindow(main).max(96) as i32;
        let font = CreateFontW(-(9 * dpi / 72), 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, wide("Segoe UI").as_ptr());
        let mono = CreateFontW(-(9 * dpi / 72), 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, wide("Consolas").as_ptr());
        let label = CreateWindowExW(0, wide("STATIC").as_ptr(),
                                    wide("Drop D2R excel .bin files or folders here.\nThe .txt files are written next to them.").as_ptr(),
                                    WS_CHILD | WS_VISIBLE | SS_CENTER, 0, 0, 0, 0, main, null_mut(), inst, null_mut());
        let button = CreateWindowExW(0, wide("BUTTON").as_ptr(), wide("Browse...").as_ptr(), WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
                                     0, 0, 0, 0, main, ID_BROWSE as *mut c_void, inst, null_mut());
        let edit = CreateWindowExW(WS_EX_CLIENTEDGE, wide("EDIT").as_ptr(), wide("").as_ptr(),
                                   WS_CHILD | WS_VISIBLE | WS_VSCROLL | WS_HSCROLL | ES_MULTILINE | ES_AUTOVSCROLL | ES_AUTOHSCROLL | ES_READONLY,
                                   0, 0, 0, 0, main, null_mut(), inst, null_mut());
        for (c, f) in [(label, font), (button, font), (edit, mono)] {
            SendMessageW(c, WM_SETFONT, f as usize, 1);
        }
        LABEL.store(label as isize, Ordering::Relaxed);
        BUTTON.store(button as isize, Ordering::Relaxed);
        EDIT.store(edit as isize, Ordering::Relaxed);
        DragAcceptFiles(main, 1);
        layout(main);
        // files dropped on the .exe itself
        convert_and_log(&args.iter().map(PathBuf::from).collect::<Vec<_>>());
        let mut msg: Msg = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
