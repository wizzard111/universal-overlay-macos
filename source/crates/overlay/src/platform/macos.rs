//! macOS host.
//!
//! * Overlay: borderless, transparent NSWindow at status-bar level that
//!   ignores the mouse and joins every Space, including full-screen apps.
//!   Pixels go to the content view's CALayer as a CGImage (Retina aware).
//! * Menu-bar item with a menu; global hotkey via Carbon RegisterEventHotKey
//!   (works without Accessibility permission).
//! * IPC: Unix socket (`--toggle`, settings app), handled on the main thread.
//!
//! Frame timing is not available on macOS (see frames/unsupported.rs).

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColor, NSMenu, NSMenuItem,
    NSScreen, NSStatusBar, NSStatusItem, NSVariableStatusItemLength, NSWindow,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize, NSString, NSTimer};
use std::cell::RefCell;
use std::ffi::c_void;
use std::io::Read;
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use uo_common::config::MonitorMode;
use uo_common::hotkey::{mac_keycode, Hotkey};
use uo_common::{ipc, SETTINGS_EXE};
use uo_overlay::core::{icon_pixels, Core, Rect};

// ---------------------------------------------------------------- C APIs

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
    fn CGColorSpaceRelease(cs: *mut c_void);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        w: usize,
        h: usize,
        bpc: usize,
        bpr: usize,
        cs: *mut c_void,
        info: u32,
    ) -> *mut c_void;
    fn CGBitmapContextCreateImage(ctx: *mut c_void) -> *mut c_void;
    fn CGContextRelease(ctx: *mut c_void);
    fn CGImageRelease(img: *mut c_void);
}

/// kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little = BGRA in
/// memory, which is exactly our premultiplied 0xAARRGGBB little-endian u32.
const BITMAP_INFO: u32 = 2 | (2 << 12);

#[repr(C)]
struct EventTypeSpec {
    class: u32,
    kind: u32,
}

#[repr(C)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

type EventHandler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: EventHandler,
        n: u32,
        list: *const EventTypeSpec,
        user: *mut c_void,
        out: *mut *mut c_void,
    ) -> i32;
    fn RegisterEventHotKey(
        code: u32,
        mods: u32,
        id: EventHotKeyID,
        target: *mut c_void,
        options: u32,
        out: *mut *mut c_void,
    ) -> i32;
    fn UnregisterEventHotKey(r: *mut c_void) -> i32;
}

const K_EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const K_EVENT_HOTKEY_PRESSED: u32 = 5;
const CMD_KEY: u32 = 0x0100;
const SHIFT_KEY: u32 = 0x0200;
const OPTION_KEY: u32 = 0x0800;
const CONTROL_KEY: u32 = 0x1000;

/// Make a CGImage from premultiplied pixels. Caller releases it.
fn cg_image(px: &[u32], w: i32, h: i32) -> *mut c_void {
    unsafe {
        let cs = CGColorSpaceCreateDeviceRGB();
        let ctx = CGBitmapContextCreate(
            px.as_ptr() as *mut c_void,
            w as usize,
            h as usize,
            8,
            w as usize * 4,
            cs,
            BITMAP_INFO,
        );
        CGColorSpaceRelease(cs);
        if ctx.is_null() {
            return std::ptr::null_mut();
        }
        let img = CGBitmapContextCreateImage(ctx); // copies the pixels
        CGContextRelease(ctx);
        img
    }
}

fn quit_app() -> ! {
    let _ = std::fs::remove_file(ipc::socket_path());
    std::process::exit(0);
}

// ---------------------------------------------------------------- target

define_class!(
    /// Receives menu actions, timer ticks and IPC commands on the main thread.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "UniversalOverlayTarget"]
    struct Target;

    impl Target {
        #[unsafe(method(toggle:))]
        fn toggle(&self, _sender: Option<&AnyObject>) {
            with_app(|a| a.toggle());
        }

        #[unsafe(method(settings:))]
        fn settings(&self, _sender: Option<&AnyObject>) {
            open_settings();
        }

        #[unsafe(method(reload:))]
        fn reload(&self, _sender: Option<&AnyObject>) {
            with_app(|a| a.reload());
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            quit_app();
        }

        #[unsafe(method(tick:))]
        fn tick(&self, _timer: Option<&AnyObject>) {
            with_app(|a| a.tick());
        }

        #[unsafe(method(frame:))]
        fn frame(&self, _timer: Option<&AnyObject>) {
            with_app(|a| a.render());
        }

        #[unsafe(method(ipc:))]
        fn ipc(&self, cmd: Option<&NSString>) {
            let cmd = cmd.map(|s| s.to_string()).unwrap_or_default();
            match cmd.trim() {
                "toggle" => {
                    with_app(|a| a.toggle());
                }
                "reload" => {
                    with_app(|a| a.reload());
                }
                "show" => {
                    with_app(|a| a.show());
                }
                "hide" => {
                    with_app(|a| a.hide());
                }
                "settings" => open_settings(),
                "quit" => quit_app(),
                _ => {}
            }
        }

        /// The app was opened again (Finder, Launchpad, Spotlight) while
        /// running: it has no windows of its own, so show the settings.
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn reopen(&self, _app: Option<&AnyObject>, _visible: objc2::runtime::Bool) -> objc2::runtime::Bool {
            open_settings();
            objc2::runtime::Bool::NO
        }
    }
);

fn open_settings() {
    let _ = std::process::Command::new(super::exe_dir().join(SETTINGS_EXE)).spawn();
}

impl Target {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

// ---------------------------------------------------------------- app

struct App {
    mtm: MainThreadMarker,
    core: Core,
    window: Retained<NSWindow>,
    target: Retained<Target>,
    status: Retained<NSStatusItem>,
    visible: bool,
    tick_timer: Option<Retained<NSTimer>>,
    frame_timer: Option<Retained<NSTimer>>,
    hotkey: *mut c_void,
    /// Last window frame (x, y, w, h); moving the window only when it
    /// changes saves WindowServer work on every graph redraw.
    frame: (f64, f64, f64, f64),
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

extern "C" fn on_hotkey(_next: *mut c_void, _event: *mut c_void, _user: *mut c_void) -> i32 {
    with_app(|a| a.toggle());
    0
}

impl App {
    fn register_hotkey(&mut self) {
        unsafe {
            if !self.hotkey.is_null() {
                UnregisterEventHotKey(self.hotkey);
                self.hotkey = std::ptr::null_mut();
            }
            let Some(hk) = Hotkey::parse(&self.core.cfg.general.hotkey) else {
                return;
            };
            let Some(code) = mac_keycode(&hk.key) else {
                return;
            };
            let mut mods = 0;
            if hk.ctrl {
                mods |= CONTROL_KEY;
            }
            if hk.alt {
                mods |= OPTION_KEY;
            }
            if hk.shift {
                mods |= SHIFT_KEY;
            }
            if hk.win {
                mods |= CMD_KEY;
            }
            let id = EventHotKeyID {
                signature: u32::from_be_bytes(*b"UOvl"),
                id: 1,
            };
            RegisterEventHotKey(
                code,
                mods,
                id,
                GetApplicationEventTarget(),
                0,
                &mut self.hotkey,
            );
        }
    }

    fn timer(&self, secs: f64, sel: Sel) -> Retained<NSTimer> {
        unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                secs,
                &self.target,
                sel,
                None,
                true,
            )
        }
    }

    fn set_timers(&mut self) {
        self.stop_timers();
        let g = &self.core.cfg.general;
        let (tick, frame) = (
            g.update_interval_ms as f64 / 1000.0,
            1.0 / g.graph_refresh_hz.max(1) as f64,
        );
        self.tick_timer = Some(self.timer(tick, sel!(tick:)));
        if self.core.cfg.graph.enabled {
            self.frame_timer = Some(self.timer(frame, sel!(frame:)));
        }
    }

    fn stop_timers(&mut self) {
        for t in [self.tick_timer.take(), self.frame_timer.take()]
            .into_iter()
            .flatten()
        {
            t.invalidate();
        }
    }

    fn show(&mut self) {
        if self.visible {
            return;
        }
        self.visible = true;
        self.core.start_frames();
        self.set_timers();
        self.tick();
        self.window.orderFrontRegardless();
    }

    fn hide(&mut self) {
        if !self.visible {
            return;
        }
        self.visible = false;
        self.stop_timers();
        self.window.orderOut(None);
        self.frame = (0.0, 0.0, 0.0, 0.0);
        self.core.on_hidden();
    }

    fn toggle(&mut self) {
        if self.visible {
            self.hide()
        } else {
            self.show()
        }
    }

    fn reload(&mut self) {
        if self.core.reload(self.visible) {
            self.register_hotkey();
        }
        if self.visible {
            self.set_timers();
            self.tick();
        }
    }

    fn tick(&mut self) {
        let ws = objc2_app_kit::NSWorkspace::sharedWorkspace();
        let pid = ws
            .frontmostApplication()
            .map(|a| a.processIdentifier() as u32);
        self.core.tick(pid.filter(|&p| p != std::process::id()));
        self.render();
    }

    fn screen(&self) -> Option<Retained<NSScreen>> {
        match self.core.cfg.general.monitor {
            MonitorMode::Foreground => NSScreen::mainScreen(self.mtm),
            MonitorMode::Primary => NSScreen::screens(self.mtm).firstObject(),
        }
    }

    fn render(&mut self) {
        if !self.visible {
            return;
        }
        let Some(screen) = self.screen() else { return };
        let f = screen.frame();
        let backing = screen.backingScaleFactor() as f32;
        // Core works top-down; give it the screen with y = 0 at the top.
        let mon = Rect {
            x: f.origin.x as i32,
            y: 0,
            w: f.size.width as i32,
            h: f.size.height as i32,
        };
        let (x, y_top) = self.core.render(mon, backing, backing);
        let cv = &self.core.canvas;
        let (wpt, hpt) = (cv.w as f64 / backing as f64, cv.h as f64 / backing as f64);
        // Cocoa: origin is bottom-left.
        let y = f.origin.y + f.size.height - y_top as f64 - hpt;
        let fr = (x as f64, y, wpt, hpt);
        if fr != self.frame {
            self.frame = fr;
            self.window.setFrame_display(
                NSRect::new(NSPoint::new(fr.0, fr.1), NSSize::new(fr.2, fr.3)),
                false,
            );
        }
        let img = cg_image(&cv.px, cv.w, cv.h);
        if img.is_null() {
            return;
        }
        if let Some(view) = self.window.contentView() {
            view.setWantsLayer(true);
            if let Some(layer) = view.layer() {
                layer.setContentsScale(backing as f64);
                unsafe {
                    let _: () = msg_send![&*layer, setContents: img as *mut AnyObject];
                }
            }
        }
        unsafe { CGImageRelease(img) };
    }
}

fn menu_item(
    mtm: MainThreadMarker,
    title: &str,
    action: Sel,
    target: &Target,
) -> Retained<NSMenuItem> {
    unsafe {
        let item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            Some(action),
            &NSString::from_str(""),
        );
        item.setTarget(Some(target));
        item
    }
}

fn status_icon(mtm: MainThreadMarker, item: &NSStatusItem) {
    // 18 pt icon drawn by our renderer, @2x for Retina.
    let size = 36;
    let px: Vec<u32> = icon_pixels(size)
        .into_iter()
        .map(|p| {
            // icon_pixels is straight alpha; CG wants premultiplied.
            let a = p >> 24;
            let m = |c: u32| c * a / 255;
            (a << 24) | (m((p >> 16) & 255) << 16) | (m((p >> 8) & 255) << 8) | m(p & 255)
        })
        .collect();
    let img = cg_image(&px, size, size);
    if let Some(button) = item.button(mtm) {
        if img.is_null() {
            button.setTitle(&NSString::from_str("UO"));
            return;
        }
        unsafe {
            let ns_image: *mut AnyObject = msg_send![objc2::class!(NSImage), alloc];
            let ns_image: *mut AnyObject = msg_send![ns_image, initWithCGImage: img as *mut AnyObject, size: NSSize::new(18.0, 18.0)];
            let _: () = msg_send![&*button, setImage: ns_image];
            let _: () = msg_send![ns_image, release];
            CGImageRelease(img);
        }
    }
}

fn spawn_ipc_listener(target: &Target) {
    let sock = ipc::socket_path();
    let _ = std::fs::remove_file(&sock);
    let Ok(listener) = UnixListener::bind(&sock) else {
        return;
    };
    // The target lives for the whole process; pass its address to the thread.
    static TARGET: AtomicUsize = AtomicUsize::new(0);
    TARGET.store(target as *const Target as usize, Ordering::Relaxed);
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let mut s = s;
            let mut buf = String::new();
            let _ = s.read_to_string(&mut buf);
            let t = TARGET.load(Ordering::Relaxed) as *const AnyObject;
            if t.is_null() {
                continue;
            }
            let cmd = NSString::from_str(buf.trim());
            unsafe {
                let _: () = msg_send![&*t, performSelectorOnMainThread: sel!(ipc:), withObject: &*cmd, waitUntilDone: false];
            }
        }
    });
}

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    for (flag, cmd) in [
        ("--quit", "quit"),
        ("--reload", "reload"),
        ("--show", "show"),
        ("--hide", "hide"),
    ] {
        if has(flag) {
            ipc::send(cmd);
            return;
        }
    }
    if has("--settings") {
        if !ipc::send("settings") {
            let _ = std::process::Command::new(super::exe_dir().join(SETTINGS_EXE)).spawn();
        }
        return;
    }
    // --toggle on a stopped overlay starts it, visible.
    let toggle = has("--toggle");
    if toggle && ipc::send("toggle") {
        return;
    }
    if ipc::is_running() {
        ipc::send("reload");
        if !has("--background") {
            ipc::send("settings");
        }
        return;
    }
    let mtm = MainThreadMarker::new().expect("must run on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory); // no Dock icon

    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0)),
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setHasShadow(false);
    window.setIgnoresMouseEvents(true);
    window.setLevel(objc2_app_kit::NSStatusWindowLevel);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );

    let target = Target::new(mtm);
    // Target also acts as the application delegate (reopen → settings).
    unsafe {
        let _: () = msg_send![&*app, setDelegate: &*target];
    }
    let status = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    status_icon(mtm, &status);
    let menu = NSMenu::new(mtm);
    menu.addItem(&menu_item(
        mtm,
        "Show / hide overlay",
        sel!(toggle:),
        &target,
    ));
    menu.addItem(&menu_item(mtm, "Settings…", sel!(settings:), &target));
    menu.addItem(&menu_item(mtm, "Reload config", sel!(reload:), &target));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&menu_item(mtm, "Exit", sel!(quit:), &target));
    status.setMenu(Some(&menu));

    unsafe {
        let spec = EventTypeSpec {
            class: K_EVENT_CLASS_KEYBOARD,
            kind: K_EVENT_HOTKEY_PRESSED,
        };
        InstallEventHandler(
            GetApplicationEventTarget(),
            on_hotkey,
            1,
            &spec,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
    }
    spawn_ipc_listener(&target);

    let core = Core::new();
    let show = core.cfg.general.show_on_start || toggle;
    let mut a = App {
        mtm,
        core,
        window,
        target,
        status,
        visible: false,
        tick_timer: None,
        frame_timer: None,
        hotkey: std::ptr::null_mut(),
        frame: (0.0, 0.0, 0.0, 0.0),
    };
    a.register_hotkey();
    let _ = &a.status;
    APP.with(|s| *s.borrow_mut() = Some(a));
    if show {
        with_app(|a| a.show());
    }
    app.run();
}
