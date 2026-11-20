pub mod board_view;
pub mod macos;
pub mod panel_view;
pub mod scanner;
pub mod server;
pub mod store;
pub mod types;

use board_view::BoardView;
use gpui_kit::assets::Assets;
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::*;
use macos::MacStatusController;
use panel_view::{
    PANEL_EXPANDED_HEIGHT, PANEL_TITLE, PANEL_WIDTH, PanelView, SharedWindowPrefs,
};
use server::{run_loopback_server, start_fs_watcher};
use std::cell::RefCell;
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use store::{JobStore, focus_coco_workspace};
use types::{BackendEvent, LOOPBACK_PORT, MonitorSnapshot};

fn notify_existing_instance(open_board: bool) -> bool {
    let addr: SocketAddr = format!("127.0.0.1:{LOOPBACK_PORT}")
        .parse()
        .expect("valid loopback addr");
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(120)) else {
        return false;
    };
    let endpoint = if open_board { "/api/board" } else { "/api/show" };
    let req = format!(
        "POST {endpoint} HTTP/1.1\r\nHost: 127.0.0.1:{LOOPBACK_PORT}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let _ = stream.write_all(req.as_bytes());
    let _ = stream.flush();
    true
}

fn open_or_focus_board(
    board_slot: &Rc<RefCell<Option<AnyWindowHandle>>>,
    store: &Entity<JobStore>,
    cx: &mut App,
) {
    if let Some(existing) = *board_slot.borrow() {
        if existing
            .update(cx, |_, window, _| {
                window.activate_window();
            })
            .is_ok()
        {
            cx.activate(true);
            return;
        }
    }

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(1240.), px(780.)), cx)),
        titlebar: Some(TitlebarOptions {
            title: Some("CoCo Job Monitor — 詳細ボード".into()),
            appears_transparent: false,
            ..Default::default()
        }),
        kind: WindowKind::Normal,
        is_resizable: true,
        is_minimizable: true,
        window_min_size: Some(size(px(860.), px(520.))),
        ..Default::default()
    };

    let store_clone = store.clone();
    if let Ok((handle, _)) = gpui_kit::open_window(options, cx, |_, cx| {
        cx.new(|cx| BoardView::new(store_clone, cx))
    }) {
        *board_slot.borrow_mut() = Some(handle);
        cx.activate(true);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let want_board = args.iter().any(|a| a == "--board");

    if notify_existing_instance(want_board) {
        return;
    }

    let application = gpui_kit::application().with_assets(Assets);
    application.run(move |cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);

        let shared_snapshot = Arc::new(RwLock::new(MonitorSnapshot::default()));
        let store = cx.new(|_| JobStore::new(shared_snapshot.clone()));
        let prefs = SharedWindowPrefs::new();
        let board_handle: Rc<RefCell<Option<AnyWindowHandle>>> = Rc::new(RefCell::new(None));

        let (tx, rx) = smol::channel::bounded::<BackendEvent>(512);
        start_fs_watcher(tx.clone());

        let tx_srv = tx.clone();
        let snap_srv = shared_snapshot.clone();
        cx.background_spawn(async move {
            let _ = run_loopback_server(tx_srv, snap_srv).await;
        })
        .detach();

        let status_ctrl = Rc::new(RefCell::new(MacStatusController::new(tx.clone())));

        let tx_board = tx.clone();
        let on_open_board: Arc<dyn Fn(&mut App) + Send + Sync> = Arc::new(move |_| {
            let _ = tx_board.try_send(BackendEvent::OpenBoardWindow);
        });

        let panel_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(1340.), px(78.)),
                size(px(PANEL_WIDTH), px(PANEL_EXPANDED_HEIGHT)),
            ))),
            titlebar: None,
            focus: false,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: true,
            app_owns_titlebar_drag: true,
            is_resizable: true,
            is_minimizable: false,
            window_min_size: Some(size(px(320.), px(50.))),
            ..Default::default()
        };

        let store_for_panel = store.clone();
        let prefs_for_panel = prefs.clone();
        let (panel_win, _) = gpui_kit::open_window(panel_options, cx, |window, cx| {
            window.set_window_title(PANEL_TITLE);
            cx.new(|cx| PanelView::new(store_for_panel, prefs_for_panel, on_open_board, cx))
        })
        .expect("Failed to open CoCo Job Monitor panel");
        let _ = panel_win;

        // Initial menu bar sync & CoCo window snap
        {
            let snap = store.read(cx).snapshot.clone();
            if let Some(ref mut ctrl) = *status_ctrl.borrow_mut() {
                ctrl.update_from_snapshot(&snap, prefs.current_menubar_mode());
                ctrl.sync_with_coco_window(&prefs);
            }
        }

        if want_board {
            open_or_focus_board(&board_handle, &store, cx);
        }

        // Async event consumer (Hooks, FSEvents, Menu bar click, Board trigger)
        let store_ev = store.clone();
        let status_ev = status_ctrl.clone();
        let board_ev = board_handle.clone();
        let prefs_ev = prefs.clone();
        cx.spawn(async move |cx| {
            while let Ok(ev) = rx.recv().await {
                cx.update(|cx| {
                    match ev {
                        BackendEvent::Hook(hook_ev) => {
                            store_ev.update(cx, |s, cx| {
                                s.apply_hook(hook_ev);
                                cx.notify();
                            });
                        }
                        BackendEvent::FsChanged => {
                            // Drain bursty FSEvents notifications cleanly
                            while let Ok(BackendEvent::FsChanged) = rx.try_recv() {}
                            store_ev.update(cx, |s, cx| {
                                s.refresh(true);
                                cx.notify();
                            });
                        }
                        BackendEvent::FocusWorkspace(cwd) => {
                            focus_coco_workspace(&cwd);
                        }
                        BackendEvent::ToggleMainPanel => {
                            if let Some(ref mut ctrl) = *status_ev.borrow_mut() {
                                ctrl.toggle_panel_visibility();
                            }
                        }
                        BackendEvent::OpenBoardWindow => {
                            if let Some(ref mut ctrl) = *status_ev.borrow_mut() {
                                ctrl.show_panel();
                            }
                            open_or_focus_board(&board_ev, &store_ev, cx);
                        }
                    }

                    let snap = store_ev.read(cx).snapshot.clone();
                    if let Some(ref mut ctrl) = *status_ev.borrow_mut() {
                        ctrl.update_from_snapshot(&snap, prefs_ev.current_menubar_mode());
                    }
                });
            }
        })
        .detach();

        // Adaptive Menu Bar Sprite Animator (sleeps 600ms when idle, 55ms..130ms when running)
        let status_anim = status_ctrl.clone();
        let prefs_anim = prefs.clone();
        cx.spawn(async move |cx| {
            loop {
                let sleep_ms = cx.update(|_| {
                    if let Some(ref mut ctrl) = *status_anim.borrow_mut() {
                        ctrl.tick_animation(prefs_anim.current_menubar_mode());
                        ctrl.current_interval_ms().clamp(50, 600)
                    } else {
                        600
                    }
                });
                cx.background_executor()
                    .timer(Duration::from_millis(sleep_ms))
                    .await;
            }
        })
        .detach();

        // 250ms CoCo window tracker + 1s heartbeat ticker + 3s fallback rescan
        let store_tick = store.clone();
        let status_tick = status_ctrl.clone();
        let prefs_tick = prefs.clone();
        cx.spawn(async move |cx| {
            let mut ticks: u64 = 0;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                ticks = ticks.wrapping_add(1);

                let do_heartbeat = ticks % 4 == 0;
                let do_rescan = ticks % 12 == 0;

                cx.update(|cx| {
                    if do_heartbeat {
                        store_tick.update(cx, |s, cx| {
                            s.refresh(do_rescan);
                            cx.notify();
                        });
                    }
                    let snap = store_tick.read(cx).snapshot.clone();
                    if let Some(ref mut ctrl) = *status_tick.borrow_mut() {
                        if do_rescan {
                            ctrl.reload_icons_if_changed();
                        }
                        if do_heartbeat {
                            ctrl.update_from_snapshot(&snap, prefs_tick.current_menubar_mode());
                        }
                        ctrl.sync_with_coco_window(&prefs_tick);
                    }
                });
            }
        })
        .detach();
    });
}
