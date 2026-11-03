mod catalog;
mod engine;
mod ledger;
mod model;
mod quota;
mod settings;

use engine::Engine;
use serde_json::{json, Value};
use settings::Settings;
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as AutostartExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

fn toggle(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false) {
            save_window(app);
            let _ = window.hide();
        } else {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}
fn save_window(app: &tauri::AppHandle) {
    if let (Some(window), Some(engine)) = (
        app.get_webview_window("main"),
        app.try_state::<Arc<Engine>>(),
    ) {
        if let (Ok(size), Ok(position), Ok(scale)) = (
            window.inner_size(),
            window.outer_position(),
            window.scale_factor(),
        ) {
            engine.persist_window(
                size.width as f64 / scale,
                size.height as f64 / scale,
                position.x,
                position.y,
            );
        }
    }
}
fn appearance(window: &tauri::WebviewWindow, settings: &Settings) {
    let _ = window.set_always_on_top(settings.always_on_top);
    let _ = window.set_zoom(settings.zoom);
    let _ = window.set_theme(match settings.theme.as_str() {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None,
    });
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{
            apply_liquid_glass, apply_vibrancy, clear_liquid_glass, clear_vibrancy,
            LiquidGlassOptions, NSGlassEffectViewStyle, NSVisualEffectMaterial,
        };
        let _ = clear_liquid_glass(window);
        let _ = clear_vibrancy(window);
        if settings.system_glass {
            let liquid = if settings.glass_style != "hud" {
                apply_liquid_glass(
                    window,
                    LiquidGlassOptions::new(if settings.glass_style == "clear" {
                        NSGlassEffectViewStyle::Clear
                    } else {
                        NSGlassEffectViewStyle::Regular
                    })
                    .radius(14.0),
                )
                .is_ok()
            } else {
                false
            };
            if !liquid {
                let _ = apply_vibrancy(window, NSVisualEffectMaterial::HudWindow, None, Some(14.0));
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        let _ = window_vibrancy::clear_acrylic(window);
        let _ = window_vibrancy::clear_mica(window);
        if settings.system_glass {
            let _ = window_vibrancy::apply_acrylic(window, Some((48, 52, 56, 90)));
        }
    }
}
fn system_settings(app: &tauri::AppHandle, settings: &Settings) -> Result<(), String> {
    if settings.start_at_login {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    }
    .map_err(|_| "开机启动设置失败")?;
    app.global_shortcut()
        .unregister_all()
        .map_err(|_| "快捷键设置失败")?;
    if !settings.shortcut.trim().is_empty() {
        app.global_shortcut()
            .register(settings.shortcut.as_str())
            .map_err(|_| "快捷键格式无效或已被其他应用占用")?;
    }
    if let Some(window) = app.get_webview_window("main") {
        appearance(&window, settings);
    }
    Ok(())
}
#[tauri::command]
fn bootstrap(engine: tauri::State<'_, Arc<Engine>>) -> Value {
    engine.bootstrap()
}
#[tauri::command]
fn get_view(
    engine: tauri::State<'_, Arc<Engine>>,
    period: String,
    group: String,
    cycle: String,
) -> Value {
    json!({"view":engine.view(&period,&group,&cycle),"status":engine.status()})
}
#[tauri::command]
fn save_settings(
    app: tauri::AppHandle,
    engine: tauri::State<'_, Arc<Engine>>,
    mut settings: Settings,
    secret: Option<String>,
) -> Result<Value, String> {
    settings.validate()?;
    let old = engine.settings.lock().unwrap().0.clone();
    if let Err(error) = system_settings(&app, &settings) {
        let _ = system_settings(&app, &old);
        return Err(error);
    }
    if let Err(error) = engine.update(settings, secret) {
        let _ = system_settings(&app, &old);
        return Err(error);
    }
    Ok(engine.bootstrap())
}
#[tauri::command]
fn refresh(engine: tauri::State<'_, Arc<Engine>>) {
    engine.collect.notify_one();
    engine.refresh_quota.notify_one();
}
#[tauri::command]
fn window_action(app: tauri::AppHandle, action: String) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("窗口不可用")?;
    match action.as_str() {
        "hide" => {
            save_window(&app);
            window.hide()
        }
        "minimize" => window.minimize(),
        "drag" => window.start_dragging(),
        _ => return Err("窗口操作无效".into()),
    }
    .map_err(|_| "窗口操作失败".into())
}
#[tauri::command]
fn open_project_link(kind: String) -> Result<(), String> {
    let url = match kind.as_str() {
        "releases" => "https://github.com/mrxiuer/token-monitor/releases",
        "issues" => "https://github.com/mrxiuer/token-monitor/issues",
        _ => "https://github.com/mrxiuer/token-monitor",
    };
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("rundll32.exe");
        command.arg("url.dll,FileProtocolHandler");
        command
    };
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    command.arg(url).spawn().map_err(|_| "无法打开浏览器")?;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            get_view,
            save_settings,
            refresh,
            window_action,
            open_project_link
        ])
        .setup(|app| {
            let engine = Engine::new(app.handle().clone()).map_err(std::io::Error::other)?;
            let settings = engine.settings.lock().unwrap().0.clone();
            app.manage(engine.clone());
            let show = MenuItem::with_id(app, "show", "显示 / 收起", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Token Monitor Duo · 后台统计中")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => toggle(app),
                    "quit" => {
                        save_window(app);
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        toggle(tray.app_handle());
                    }
                })
                .build(app)?;
            if let Some(window) = app.get_webview_window("main") {
                if std::env::var_os("TOKEN_MONITOR_DUO_TEST").is_some() {
                    let _ = window.set_title("Token Monitor Duo · 隔离测试");
                }
                let _ = window.set_size(tauri::LogicalSize::new(settings.width, settings.height));
                if let (Some(x), Some(y)) = (settings.x, settings.y) {
                    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
                }
                appearance(&window, &settings);
                let _ = window.show();
            }
            if !settings.shortcut.is_empty() {
                let _ = app.global_shortcut().register(settings.shortcut.as_str());
            }
            engine.start();
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::Focused(true) = event {
                if let Some(engine) = window.app_handle().try_state::<Arc<Engine>>() {
                    engine.refresh_quota.notify_one();
                }
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                save_window(window.app_handle());
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("Token Monitor Duo failed to start")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                save_window(app);
            }
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } = event
            {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        });
}
