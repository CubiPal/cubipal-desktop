// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    use_software_rendering_without_gpu();

    cubipal_desktop_lib::run()
}

/// WebKitGTK draws the window with the GPU by default. On Linux machines with
/// no GPU device (the ChromeOS Linux VM, many VMs and remote desktops) that
/// fails silently and the window stays blank, so fall back to software
/// rendering there. Values the user has already set are left alone.
#[cfg(target_os = "linux")]
fn use_software_rendering_without_gpu() {
    if std::path::Path::new("/dev/dri").exists() {
        return;
    }
    for var in [
        "WEBKIT_DISABLE_DMABUF_RENDERER",
        "WEBKIT_DISABLE_COMPOSITING_MODE",
    ] {
        if std::env::var_os(var).is_none() {
            // SAFETY: called first thing in main, before Tauri, WebKit or the
            // async runtime start any threads that could read the environment.
            unsafe { std::env::set_var(var, "1") };
        }
    }
}
