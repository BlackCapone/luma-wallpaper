//! Windows 10: окно под иконками рабочего стола, пауза, автозапуск.
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
        UI::{
            Shell::{SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN},
            WindowsAndMessaging::*,
        },
    },
};
use winreg::{enums::*, RegKey};

unsafe extern "system" fn enum_proc(top: HWND, lparam: LPARAM) -> BOOL {
    if FindWindowExW(top, None, w!("SHELLDLL_DefView"), None).is_ok() {
        if let Ok(w) = FindWindowExW(None, top, w!("WorkerW"), None) {
            *(lparam.0 as *mut HWND) = w;
        }
    }
    TRUE
}

/// Крепит окно к рабочему столу. x, y — смещение относительно верхнего левого угла
/// виртуального экрана (всех мониторов), w, h — размер монитора в пикселях.
pub fn attach(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) -> Result<()> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), None)?;
        let mut _res = 0usize;
        SendMessageTimeoutW(progman, 0x052C, WPARAM(0xD), LPARAM(1), SMTO_NORMAL, 1000, Some(&mut _res));

        let mut workerw = HWND::default();
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut workerw as *mut _ as isize));
        // fallback для Windows 11 24H2: WorkerW внутри Progman
        let parent = if workerw.is_invalid() { progman } else { workerw };

        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let style = (style & !(WS_POPUP.0 | WS_CAPTION.0 | WS_THICKFRAME.0)) | WS_CHILD.0;
        SetWindowLongPtrW(hwnd, GWL_STYLE, style as isize);
        SetParent(hwnd, parent)?;
        SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW)?;
    }
    Ok(())
}

/// Полноэкранное приложение / игра / режим презентации.
pub fn fullscreen_busy() -> bool {
    unsafe {
        match SHQueryUserNotificationState() {
            Ok(s) => s == QUNS_BUSY || s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE,
            Err(_) => false,
        }
    }
}

/// Работа от батареи (ноутбук не в розетке).
pub fn on_battery() -> bool {
    unsafe {
        let mut s = SYSTEM_POWER_STATUS::default();
        GetSystemPowerStatus(&mut s).is_ok() && s.ACLineStatus == 0
    }
}

/// Для режима `unpack`: вывод в консоль, из которой запущен exe (в релизе у него нет своей).
pub fn attach_console() {
    unsafe {
        let _ = windows::Win32::System::Console::AttachConsole(windows::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}

pub fn set_autostart(on: bool) -> std::io::Result<()> {
    let run = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\Run", KEY_SET_VALUE)?;
    if on {
        let exe = std::env::current_exe()?;
        run.set_value("LumaWallpaper", &format!("\"{}\" --tray", exe.display()))
    } else {
        let _ = run.delete_value("LumaWallpaper");
        Ok(())
    }
}
