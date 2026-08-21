pub mod ai;
pub mod commands;
pub mod db;
pub mod git;
pub mod models;
pub mod opencode;
pub mod reports;

use std::sync::Mutex;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    ActivationPolicy, AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

const PANEL_LABEL: &str = "panel";
const TRAY_ID: &str = "breadcrumbs-tray";
const PANEL_WIDTH: f64 = 380.0;
const PANEL_HEIGHT: f64 = 560.0;

fn ensure_panel(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    if let Some(w) = app.get_webview_window(PANEL_LABEL) {
        return Ok(w);
    }
    let window = WebviewWindowBuilder::new(
        app,
        PANEL_LABEL,
        WebviewUrl::App("index.html".into()),
    )
    .title("Breadcrumbs")
    .inner_size(PANEL_WIDTH, PANEL_HEIGHT)
    .decorations(false)
    .transparent(true)
    .resizable(false)
    .visible(false)
    .skip_taskbar(true)
    .always_on_top(true)
    .accept_first_mouse(true)
    .shadow(false)
    .build()?;

    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
        apply_vibrancy(
            &window,
            NSVisualEffectMaterial::Popover,
            Some(NSVisualEffectState::FollowsWindowActiveState),
            Some(12.0),
        )
        .map_err(|e| e.to_string())
        .ok();
    }

    Ok(window)
}

fn position_near_tray(window: &WebviewWindow, rect: &tauri::Rect) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let (rx, ry, rw, rh) = match (rect.position, rect.size) {
        (tauri::Position::Physical(p), tauri::Size::Physical(s)) => {
            (p.x as f64, p.y as f64, s.width as f64, s.height as f64)
        }
        (tauri::Position::Logical(p), tauri::Size::Logical(s)) => {
            (p.x * scale, p.y * scale, s.width * scale, s.height * scale)
        }
        _ => return,
    };
    let panel_w = PANEL_WIDTH * scale;
    let x = rx + (rw / 2.0) - (panel_w / 2.0);
    let y = ry + rh + 6.0;
    window
        .set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
        .ok();
}

fn show_panel(app: &AppHandle) {
    match ensure_panel(app) {
        Ok(window) => {
            if let Some(tray) = app.tray_by_id(TRAY_ID) {
                if let Ok(Some(rect)) = tray.rect() {
                    position_near_tray(&window, &rect);
                }
            }
            window.show().ok();
            window.set_focus().ok();
            commands::spawn_sync(app.clone());
        }
        Err(e) => eprintln!("failed to create panel: {e}"),
    }
}

fn toggle_panel(app: &AppHandle) {
    match app.get_webview_window(PANEL_LABEL) {
        Some(w) if w.is_visible().unwrap_or(false) => {
            w.hide().ok();
        }
        _ => show_panel(app),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            app.set_activation_policy(ActivationPolicy::Accessory);

            let data_dir = app.path().app_data_dir()?;
            let conn = db::open_diary(&data_dir)
                .map_err(|e| format!("failed to open diary.db: {e}"))?;
            app.manage(commands::AppState(Mutex::new(conn)));

            let open_i =
                MenuItem::with_id(app, "open", "Open Breadcrumbs", true, None::<&str>)?;
            let sync_i = MenuItem::with_id(app, "sync", "Sync now", true, None::<&str>)?;
            let report_i =
                MenuItem::with_id(app, "report", "Copy today's report", true, None::<&str>)?;
            let quit_i =
                MenuItem::with_id(app, "quit", "Quit Breadcrumbs", true, None::<&str>)?;

            let menu = Menu::with_items(
                app,
                &[
                    &open_i,
                    &PredefinedMenuItem::separator(app)?,
                    &sync_i,
                    &report_i,
                    &PredefinedMenuItem::separator(app)?,
                    &quit_i,
                ],
            )?;

            TrayIconBuilder::with_id(TRAY_ID)
                .icon(
                    tauri::image::Image::from_bytes(include_bytes!(
                        "../icons/tray-template.png"
                    ))
                    .expect("failed to load tray icon"),
                )
                .icon_as_template(true)
                .tooltip("Breadcrumbs")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "open" => show_panel(app),
                    "sync" => commands::spawn_sync(app.clone()),
                    "report" => commands::copy_today_report(app.clone()),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_panel(tray.app_handle());
                    }
                })
                .build(app)?;

            // initial sync at boot so the diary is warm
            commands::spawn_sync(app.handle().clone());

            // periodic background sync every 5 minutes
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(300));
                commands::spawn_sync(handle.clone());
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::sync_now,
            commands::get_timeline,
            commands::generate_report,
            commands::get_settings,
            commands::set_ai_settings,
            commands::list_ollama_models,
            commands::enhance_report
        ])
        .on_window_event(|window, event| {
            if window.label() == PANEL_LABEL {
                if let WindowEvent::Focused(false) = event {
                    window.hide().ok();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
