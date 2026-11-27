#[derive(Debug, Clone, Copy)]
pub enum TrayAction {
    ShowWindow,
    OpenSitesFolder,
    QuitApp,
}

#[cfg(windows)]
pub fn startup_command_value() -> Option<String> {
    std::env::current_exe()
        .ok()
        .map(|p| format!("\"{}\"", p.to_string_lossy()))
}

#[cfg(windows)]
pub fn is_launch_at_startup_enabled() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let out = std::process::Command::new("reg.exe")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            "DockPanel Desktop",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    matches!(out, Ok(o) if o.status.success())
}

#[cfg(not(windows))]
pub fn is_launch_at_startup_enabled() -> bool {
    false
}

#[cfg(windows)]
pub fn set_launch_at_startup(enabled: bool) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    if enabled {
        let exe_cmd = startup_command_value()
            .ok_or_else(|| "Could not resolve executable path".to_string())?;
        let out = std::process::Command::new("reg.exe")
            .args([
                "add",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "DockPanel Desktop",
                "/t",
                "REG_SZ",
                "/d",
                &exe_cmd,
                "/f",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Failed updating Windows startup registry: {}", e))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
    } else {
        let _ = std::process::Command::new("reg.exe")
            .args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "DockPanel Desktop",
                "/f",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn set_launch_at_startup(_enabled: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
mod win_tray {
    use super::TrayAction;
    use std::ffi::c_void;
    use std::sync::OnceLock;
    use tao::event_loop::EventLoopProxy;

    const WM_APP: u32 = 0x8000;
    const WM_TRAY_CALLBACK: u32 = WM_APP + 42;
    const WM_COMMAND: u32 = 0x0111;
    const WM_CLOSE: u32 = 0x0010;
    const WM_DESTROY: u32 = 0x0002;
    const WM_LBUTTONUP: u32 = 0x0202;
    const WM_LBUTTONDBLCLK: u32 = 0x0203;
    const WM_RBUTTONUP: u32 = 0x0205;

    const NIM_ADD: u32 = 0x00000000;
    const NIM_DELETE: u32 = 0x00000002;
    const NIF_MESSAGE: u32 = 0x00000001;
    const NIF_ICON: u32 = 0x00000002;
    const NIF_TIP: u32 = 0x00000004;

    const MF_STRING: u32 = 0x00000000;
    const MF_SEPARATOR: u32 = 0x00000800;
    const TPM_RIGHTALIGN: u32 = 0x0008;
    const TPM_BOTTOMALIGN: u32 = 0x0020;
    const TPM_NONOTIFY: u32 = 0x0080;
    const TPM_RETURNCMD: u32 = 0x0100;

    const ID_SHOW: usize = 1001;
    const ID_OPEN_SITES: usize = 1002;
    const ID_QUIT: usize = 1005;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct WNDCLASSW {
        style: u32,
        lpfnWndProc: unsafe extern "system" fn(isize, u32, usize, isize) -> isize,
        cbClsExtra: i32,
        cbWndExtra: i32,
        hInstance: isize,
        hIcon: isize,
        hCursor: isize,
        hbrBackground: isize,
        lpszMenuName: *const u16,
        lpszClassName: *const u16,
    }

    #[repr(C)]
    #[allow(non_snake_case)]
    struct POINT {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    #[allow(non_snake_case)]
    struct MSG {
        hwnd: isize,
        message: u32,
        wParam: usize,
        lParam: isize,
        time: u32,
        pt: POINT,
    }

    #[repr(C)]
    #[allow(non_snake_case)]
    struct GUID {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    #[repr(C)]
    #[allow(non_snake_case)]
    struct NOTIFYICONDATAW {
        cbSize: u32,
        hWnd: isize,
        uID: u32,
        uFlags: u32,
        uCallbackMessage: u32,
        hIcon: isize,
        szTip: [u16; 128],
        dwState: u32,
        dwStateMask: u32,
        szInfo: [u16; 256],
        uTimeoutOrVersion: u32,
        szInfoTitle: [u16; 64],
        dwInfoFlags: u32,
        guidItem: GUID,
        hBalloonIcon: isize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(lpModuleName: *const u16) -> isize;
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassW(lpWndClass: *const WNDCLASSW) -> u16;
        fn CreateWindowExW(
            dwExStyle: u32,
            lpClassName: *const u16,
            lpWindowName: *const u16,
            dwStyle: u32,
            X: i32,
            Y: i32,
            nWidth: i32,
            nHeight: i32,
            hWndParent: isize,
            hMenu: isize,
            hInstance: isize,
            lpParam: *const c_void,
        ) -> isize;
        fn DestroyWindow(hWnd: isize) -> i32;
        fn DefWindowProcW(hWnd: isize, msg: u32, wParam: usize, lParam: isize) -> isize;
        fn GetMessageW(lpMsg: *mut MSG, hWnd: isize, wMsgFilterMin: u32, wMsgFilterMax: u32) -> i32;
        fn TranslateMessage(lpMsg: *const MSG) -> i32;
        fn DispatchMessageW(lpMsg: *const MSG) -> isize;
        fn PostMessageW(hWnd: isize, msg: u32, wParam: usize, lParam: isize) -> i32;
        fn PostQuitMessage(nExitCode: i32);
        fn LoadIconW(hInstance: isize, lpIconName: *const u16) -> isize;
        fn CreatePopupMenu() -> isize;
        fn AppendMenuW(hMenu: isize, uFlags: u32, uIDNewItem: usize, lpNewItem: *const u16) -> i32;
        fn TrackPopupMenu(
            hMenu: isize,
            uFlags: u32,
            x: i32,
            y: i32,
            nReserved: i32,
            hWnd: isize,
            prcRect: *const c_void,
        ) -> i32;
        fn DestroyMenu(hMenu: isize) -> i32;
        fn GetCursorPos(lpPoint: *mut POINT) -> i32;
        fn SetForegroundWindow(hWnd: isize) -> i32;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn Shell_NotifyIconW(dwMessage: u32, lpData: *mut NOTIFYICONDATAW) -> i32;
    }

    static TRAY_PROXY: OnceLock<EventLoopProxy<TrayAction>> = OnceLock::new();

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn emit_action(action: TrayAction) {
        if let Some(proxy) = TRAY_PROXY.get() {
            let _ = proxy.send_event(action);
        }
    }

    unsafe extern "system" fn tray_wnd_proc(
        hwnd: isize,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        match msg {
            WM_TRAY_CALLBACK => {
                let event_code = (lparam as u32) & 0xFFFF;
                if event_code == WM_LBUTTONUP || event_code == WM_LBUTTONDBLCLK {
                    emit_action(TrayAction::ShowWindow);
                } else if event_code == WM_RBUTTONUP {
                    let hmenu = CreatePopupMenu();
                    if hmenu != 0 {
                        let lbl_show = to_wide("Show DockPanel Desktop");
                        let lbl_sites = to_wide("Open Sites Folder");
                        let lbl_quit = to_wide("Quit DockPanel Desktop");
                        AppendMenuW(hmenu, MF_STRING, ID_SHOW, lbl_show.as_ptr());
                        AppendMenuW(hmenu, MF_STRING, ID_OPEN_SITES, lbl_sites.as_ptr());
                        AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
                        AppendMenuW(hmenu, MF_STRING, ID_QUIT, lbl_quit.as_ptr());

                        let mut pt = POINT { x: 0, y: 0 };
                        GetCursorPos(&mut pt);
                        SetForegroundWindow(hwnd);
                        let cmd = TrackPopupMenu(
                            hmenu,
                            TPM_RIGHTALIGN | TPM_BOTTOMALIGN | TPM_RETURNCMD | TPM_NONOTIFY,
                            pt.x,
                            pt.y,
                            0,
                            hwnd,
                            std::ptr::null(),
                        );
                        DestroyMenu(hmenu);
                        match cmd as usize {
                            ID_SHOW => emit_action(TrayAction::ShowWindow),
                            ID_OPEN_SITES => emit_action(TrayAction::OpenSitesFolder),
                            ID_QUIT => emit_action(TrayAction::QuitApp),
                            _ => {}
                        }
                    }
                }
                0
            }
            WM_COMMAND => {
                match wparam & 0xFFFF {
                    ID_SHOW => emit_action(TrayAction::ShowWindow),
                    ID_OPEN_SITES => emit_action(TrayAction::OpenSitesFolder),
                    ID_QUIT => emit_action(TrayAction::QuitApp),
                    _ => {}
                }
                0
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    pub struct TrayHandle {
        hwnd: isize,
    }

    impl TrayHandle {
        pub fn shutdown(&self) {
            if self.hwnd != 0 {
                unsafe {
                    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
                    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                    nid.hWnd = self.hwnd;
                    nid.uID = 1;
                    Shell_NotifyIconW(NIM_DELETE, &mut nid);
                    PostMessageW(self.hwnd, WM_CLOSE, 0, 0);
                }
            }
        }
    }

    impl Drop for TrayHandle {
        fn drop(&mut self) {
            self.shutdown();
        }
    }

    pub fn start_tray(proxy: EventLoopProxy<TrayAction>) -> Option<TrayHandle> {
        let _ = TRAY_PROXY.set(proxy);
        let (tx, rx) = std::sync::mpsc::channel::<Option<isize>>();

        std::thread::spawn(move || unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());
            let class_name = to_wide("DockPanelDesktopTrayClass");
            let wnd_name = to_wide("DockPanelDesktopTrayWindow");

            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: tray_wnd_proc,
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinst,
                hIcon: 0,
                hCursor: 0,
                hbrBackground: 0,
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };
            RegisterClassW(&wc);

            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                wnd_name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                hinst,
                std::ptr::null(),
            );
            if hwnd == 0 {
                let _ = tx.send(None);
                return;
            }

            // Resource ID 1 is embedded by winres in build.rs; fallback to IDI_APPLICATION (32512).
            let mut hicon = LoadIconW(hinst, 1usize as *const u16);
            if hicon == 0 {
                hicon = LoadIconW(0, 32512usize as *const u16);
            }

            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.uCallbackMessage = WM_TRAY_CALLBACK;
            nid.hIcon = hicon;
            let tip = "DockPanel Desktop — Local Web Engine";
            for (i, ch) in tip.encode_utf16().take(127).enumerate() {
                nid.szTip[i] = ch;
            }

            if Shell_NotifyIconW(NIM_ADD, &mut nid) == 0 {
                DestroyWindow(hwnd);
                let _ = tx.send(None);
                return;
            }

            let _ = tx.send(Some(hwnd));

            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, 0, 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });

        rx.recv().ok().flatten().map(|hwnd| TrayHandle { hwnd })
    }
}

#[cfg(not(windows))]
mod win_tray {
    use super::TrayAction;
    use tao::event_loop::EventLoopProxy;

    pub struct TrayHandle;
    impl TrayHandle {
        pub fn shutdown(&self) {}
    }
    pub fn start_tray(_proxy: EventLoopProxy<TrayAction>) -> Option<TrayHandle> {
        None
    }
}

pub use win_tray::start_tray;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_startup_command_value_quotes_path() {
        #[cfg(windows)]
        {
            let v = startup_command_value().unwrap();
            assert!(v.starts_with('"') && v.ends_with('"'));
        }
    }
}
