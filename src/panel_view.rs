use crate::macos::{
    AlertSound, DockMode, MenuBarMode, apply_panel_level, open_icons_directory, play_alert_sound,
};
use crate::store::{JobStore, focus_coco_workspace, open_chrome_dashboard};
use crate::types::{JobItem, JobStatus, LOOPBACK_PORT, format_duration_ja};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

pub const PANEL_TITLE: &str = "CoCo Job Monitor Panel";
pub const PANEL_WIDTH: f32 = 368.0;
pub const PANEL_EXPANDED_HEIGHT: f32 = 580.0;
pub const PANEL_PILL_HEIGHT: f32 = 54.0;

pub struct SharedWindowPrefs {
    pub dock_mode: AtomicU8,
    pub menubar_mode: AtomicU8,
    pub pinned_on_top: AtomicBool,
    pub mini_pill: AtomicBool,
}

impl SharedWindowPrefs {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            dock_mode: AtomicU8::new(DockMode::TopRight as u8),
            menubar_mode: AtomicU8::new(MenuBarMode::IconAndCounts as u8),
            pinned_on_top: AtomicBool::new(true),
            mini_pill: AtomicBool::new(false),
        })
    }

    pub fn current_dock(&self) -> DockMode {
        DockMode::from_u8(self.dock_mode.load(Ordering::Relaxed))
    }

    pub fn current_menubar_mode(&self) -> MenuBarMode {
        MenuBarMode::from_u8(self.menubar_mode.load(Ordering::Relaxed))
    }
}

pub struct PanelView {
    store: Entity<JobStore>,
    prefs: Arc<SharedWindowPrefs>,
    show_completed: bool,
    on_open_board: Arc<dyn Fn(&mut App) + Send + Sync>,
}

impl PanelView {
    pub fn new(
        store: Entity<JobStore>,
        prefs: Arc<SharedWindowPrefs>,
        on_open_board: Arc<dyn Fn(&mut App) + Send + Sync>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        Self {
            store,
            prefs,
            show_completed: true,
            on_open_board,
        }
    }

    fn toggle_mini_pill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = !self.prefs.mini_pill.load(Ordering::Relaxed);
        self.prefs.mini_pill.store(next, Ordering::Relaxed);
        let h = if next {
            PANEL_PILL_HEIGHT
        } else {
            PANEL_EXPANDED_HEIGHT
        };
        window.resize(size(px(PANEL_WIDTH), px(h)));
        cx.notify();
    }

    fn cycle_dock_mode(&mut self, cx: &mut Context<Self>) {
        let next = match self.prefs.current_dock() {
            DockMode::TopRight => DockMode::BottomRight,
            DockMode::BottomRight => DockMode::Manual,
            DockMode::Manual => DockMode::TopRight,
        };
        self.prefs.dock_mode.store(next as u8, Ordering::Relaxed);
        cx.notify();
    }

    fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        let next = !self.prefs.pinned_on_top.load(Ordering::Relaxed);
        self.prefs.pinned_on_top.store(next, Ordering::Relaxed);
        apply_panel_level(next);
        cx.notify();
    }

    fn cycle_menubar_mode(&mut self, cx: &mut Context<Self>) {
        let next = self.prefs.current_menubar_mode().next();
        self.prefs.menubar_mode.store(next as u8, Ordering::Relaxed);
        cx.notify();
    }

    fn render_mini_pill(&self, cx: &mut Context<Self>) -> AnyElement {
        let snap = self.store.read(cx).snapshot.clone();
        let top_active = snap
            .jobs
            .iter()
            .find(|j| j.status.is_attention() || j.status.is_active())
            .cloned();

        h_flex()
            .id("mini-pill-bar")
            .size_full()
            .px_3()
            .bg(rgb(0x0b1120))
            .border_1()
            .border_color(if snap.waiting_count > 0 {
                rgb(0xf59e0b)
            } else if snap.running_count > 0 {
                rgb(0x29b5e8)
            } else {
                rgb(0x26334d)
            })
            .items_center()
            .justify_between()
            .gap_2()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.prefs
                        .dock_mode
                        .store(DockMode::Manual as u8, Ordering::Relaxed);
                    window.start_window_move();
                    cx.notify();
                }),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .overflow_hidden()
                    .when(snap.running_count > 0, |el| {
                        el.child(Spinner::new().xsmall().color(rgb(0x29b5e8).into()))
                    })
                    .child(
                        div()
                            .px_1p5()
                            .py_0p5()
                            .rounded_md()
                            .bg(if snap.waiting_count > 0 {
                                rgb(0x78350f)
                            } else {
                                rgb(0x172554)
                            })
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(if snap.waiting_count > 0 {
                                rgb(0xfcd34d)
                            } else {
                                rgb(0x38bdf8)
                            })
                            .child(format!(
                                "稼働{} · 待ち{}",
                                snap.running_count, snap.waiting_count
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0xe2e8f0))
                            .truncate()
                            .max_w(px(175.))
                            .child(if let Some(j) = top_active {
                                format!("{} ({}前)", j.title, format_duration_ja(j.heartbeat_secs))
                            } else if snap.unseen_completed_count > 0 {
                                format!("新着完了 {}件（未確認）", snap.unseen_completed_count)
                            } else {
                                "稼働・未確認ゼロ".to_string()
                            }),
                    ),
            )
            .child(
                div()
                    .id("expand-from-pill")
                    .px_2()
                    .py_0p5()
                    .rounded_md()
                    .bg(rgb(0x1e293b))
                    .hover(|s| s.bg(rgb(0x334155)))
                    .cursor_pointer()
                    .text_xs()
                    .text_color(rgb(0xf8fafc))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_mini_pill(window, cx);
                    }))
                    .child("展開"),
            )
            .into_any_element()
    }

    fn render_job_card(&self, idx: usize, job: &JobItem, cx: &mut Context<Self>) -> AnyElement {
        let (accent, card_bg, badge_bg, badge_fg) = match job.status {
            JobStatus::WaitingPermission | JobStatus::WaitingInput => {
                (rgb(0xf59e0b), rgb(0x231c12), rgb(0x78350f), rgb(0xfde68a))
            }
            JobStatus::Running => (rgb(0x29b5e8), rgb(0x131c2e), rgb(0x0c4a6e), rgb(0x7dd3fc)),
            JobStatus::Stalled => (rgb(0xef4444), rgb(0x241419), rgb(0x7f1d1d), rgb(0xfecaca)),
            JobStatus::Error => (rgb(0xef4444), rgb(0x1e151a), rgb(0x7f1d1d), rgb(0xfca5a5)),
            JobStatus::Completed => (rgb(0x10b981), rgb(0x111f1d), rgb(0x064e3b), rgb(0x6ee7b7)),
        };

        let cwd = job.working_directory.clone();
        let sid_for_click = job.session_id.clone();
        let sid_for_dismiss = job.session_id.clone();
        let is_finished = matches!(job.status, JobStatus::Completed | JobStatus::Error);
        let active_subs: Vec<_> = job.subagents.iter().filter(|s| s.active).cloned().collect();
        let heartbeat_fresh = job.status == JobStatus::Running && job.heartbeat_secs <= 15;

        v_flex()
            .id(("panel-job", idx))
            .w_full()
            .p_2p5()
            .gap_1p5()
            .rounded_lg()
            .bg(card_bg)
            .border_1()
            .border_color(if job.status.is_attention() || job.unseen_done {
                accent
            } else {
                rgb(0x24324a)
            })
            .hover(|s| s.border_color(rgb(0x38bdf8)))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                focus_coco_workspace(&cwd);
                if is_finished {
                    this.store.update(cx, |s, cx| {
                        s.dismiss_done(&sid_for_click);
                        cx.notify();
                    });
                }
            }))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_1p5()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1p5()
                            .child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(badge_bg)
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(badge_fg)
                                    .child(job.status.label()),
                            )
                            .child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(rgb(0x0b1120))
                                    .text_xs()
                                    .text_color(rgb(0x93c5fd))
                                    .child(job.workspace_name.clone()),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1p5()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(if heartbeat_fresh {
                                        FontWeight::BOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(if heartbeat_fresh {
                                        rgb(0x34d399)
                                    } else if job.status == JobStatus::Stalled {
                                        rgb(0xf87171)
                                    } else {
                                        rgb(0x94a3b8)
                                    })
                                    .child(if is_finished {
                                        format!(
                                            "{}前 (所要{})",
                                            format_duration_ja(job.heartbeat_secs),
                                            format_duration_ja(job.elapsed_secs)
                                        )
                                    } else {
                                        format!(
                                            "動作 {}前 / 計{}",
                                            format_duration_ja(job.heartbeat_secs),
                                            format_duration_ja(job.elapsed_secs)
                                        )
                                    }),
                            )
                            .when(is_finished || job.status == JobStatus::Stalled, |el| {
                                el.child(
                                    div()
                                        .id(("dismiss-done", idx))
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(rgb(0x1e293b))
                                        .hover(|s| s.bg(rgb(0x334155)))
                                        .text_xs()
                                        .text_color(rgb(0xcbd5e1))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.store.update(cx, |s, cx| {
                                                s.dismiss_done(&sid_for_dismiss);
                                                cx.notify();
                                            });
                                        }))
                                        .child("×"),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(0xf8fafc))
                    .child(job.title.clone()),
            )
            .when(job.status != JobStatus::Completed, |el| {
                el.child(
                    h_flex()
                        .items_center()
                        .gap_1p5()
                        .when(job.status == JobStatus::Running, |r| {
                            r.child(Spinner::new().xsmall().color(accent.into()))
                        })
                        .child(
                            div()
                                .text_xs()
                                .text_color(if job.status.is_attention() {
                                    rgb(0xfde68a)
                                } else {
                                    rgb(0xcbd5e1)
                                })
                                .truncate()
                                .child(job.phase.short_text()),
                        ),
                )
            })
            .when_some(job.todo.clone(), |el, todo| {
                let ratio = todo.ratio();
                el.child(
                    v_flex()
                        .w_full()
                        .gap_1()
                        .mt_0p5()
                        .child(
                            div()
                                .w_full()
                                .h(px(5.))
                                .rounded_full()
                                .bg(rgb(0x090d16))
                                .overflow_hidden()
                                .child(
                                    div()
                                        .w(relative(ratio))
                                        .h_full()
                                        .rounded_full()
                                        .bg(if ratio >= 1.0 {
                                            rgb(0x10b981)
                                        } else {
                                            rgb(0x29b5e8)
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(0x94a3b8))
                                .truncate()
                                .child(match todo.current_task {
                                    Some(ref t) if todo.completed < todo.total => {
                                        format!("Todo {}/{} · {}", todo.completed, todo.total, t)
                                    }
                                    _ => format!("Todo {}/{} 完了", todo.completed, todo.total),
                                }),
                        ),
                )
            })
            .when(!active_subs.is_empty(), |el| {
                el.child(
                    v_flex()
                        .w_full()
                        .mt_0p5()
                        .pt_1()
                        .border_t_1()
                        .border_color(rgb(0x1e293b))
                        .gap_0p5()
                        .children(active_subs.into_iter().take(3).map(|sub| {
                            div()
                                .text_xs()
                                .text_color(rgb(0x7dd3fc))
                                .truncate()
                                .child(format!("↳ {} ({})", sub.title, sub.current_step))
                        })),
                )
            })
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .text_xs()
                    .text_color(rgb(0x64748b))
                    .child(job.model.clone())
                    .child({
                        let mut parts = Vec::new();
                        if job.context_window > 0 {
                            parts.push(format!(
                                "{}k/{}k",
                                job.context_tokens / 1000,
                                job.context_window / 1000
                            ));
                        }
                        if job.diff_files > 0 {
                            parts.push(format!("+{} -{}", job.diff_added, job.diff_removed));
                        }
                        parts.join(" · ")
                    }),
            )
            .into_any_element()
    }
}

impl Render for PanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.prefs.mini_pill.load(Ordering::Relaxed) {
            return self.render_mini_pill(cx);
        }

        let (snap, notify_on, sound_on) = {
            let st = self.store.read(cx);
            (st.snapshot.clone(), st.notify_enabled, st.sound_enabled)
        };
        let dock = self.prefs.current_dock();
        let menubar_mode = self.prefs.current_menubar_mode();
        let pinned = self.prefs.pinned_on_top.load(Ordering::Relaxed);

        let mut active_jobs = Vec::new();
        let mut unseen_done_jobs = Vec::new();
        for job in &snap.jobs {
            if matches!(job.status, JobStatus::Completed | JobStatus::Error) {
                if job.unseen_done {
                    unseen_done_jobs.push(job.clone());
                }
            } else {
                active_jobs.push(job.clone());
            }
        }

        let open_board_cb = self.on_open_board.clone();

        v_flex()
            .size_full()
            .bg(rgb(0x0b1120))
            .text_color(rgb(0xf1f5f9))
            .border_1()
            .border_color(if snap.waiting_count > 0 {
                rgb(0xf59e0b)
            } else {
                rgb(0x24324a)
            })
            // Custom draggable titlebar
            .child(
                v_flex()
                    .id("panel-drag-header")
                    .w_full()
                    .px_3()
                    .pt_2p5()
                    .pb_2()
                    .gap_2()
                    .bg(rgb(0x0f172a))
                    .border_b_1()
                    .border_color(rgb(0x1e293b))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.prefs
                                .dock_mode
                                .store(DockMode::Manual as u8, Ordering::Relaxed);
                            window.start_window_move();
                            cx.notify();
                        }),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .w(px(8.))
                                            .h(px(8.))
                                            .rounded_full()
                                            .bg(if snap.waiting_count > 0 {
                                                rgb(0xf59e0b)
                                            } else if snap.running_count > 0 {
                                                rgb(0x29b5e8)
                                            } else {
                                                rgb(0x10b981)
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::BOLD)
                                            .child("CoCo Job Monitor"),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_1()
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .child(header_chip(
                                        "btn-board",
                                        "詳細ボード",
                                        true,
                                        move |_, _, cx| {
                                            open_board_cb(cx);
                                        },
                                    ))
                                    .child(header_chip(
                                        "btn-web",
                                        "Chrome",
                                        false,
                                        move |_, _, _| {
                                            open_chrome_dashboard(LOOPBACK_PORT);
                                        },
                                    ))
                                    .child(header_chip(
                                        "btn-pill",
                                        "最小化",
                                        false,
                                        cx.listener(|this, _, window, cx| {
                                            this.toggle_mini_pill(window, cx);
                                        }),
                                    )),
                            ),
                    )
                    // Control strip
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_1()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_1()
                                    .child(header_chip(
                                        "btn-dock",
                                        dock.label(),
                                        dock != DockMode::Manual,
                                        cx.listener(|this, _, _, cx| {
                                            this.cycle_dock_mode(cx);
                                        }),
                                    ))
                                    .child(header_chip(
                                        "btn-pin",
                                        if pinned { "最前面ON" } else { "最前面OFF" },
                                        pinned,
                                        cx.listener(|this, _, _, cx| {
                                            this.toggle_pin(cx);
                                        }),
                                    ))
                                    .child(header_chip(
                                        "btn-menubar-mode",
                                        menubar_mode.label(),
                                        menubar_mode != MenuBarMode::TextOnly,
                                        cx.listener(|this, _, _, cx| {
                                            this.cycle_menubar_mode(cx);
                                        }),
                                    ))
                                    .child(header_chip(
                                        "btn-icons-dir",
                                        "GIF",
                                        false,
                                        |_, _, _| {
                                            open_icons_directory();
                                        },
                                    )),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_1()
                                    .child(header_chip(
                                        "btn-notify",
                                        if notify_on { "通知ON" } else { "通知OFF" },
                                        notify_on,
                                        cx.listener(|this, _, _, cx| {
                                            this.store.update(cx, |s, cx| {
                                                s.notify_enabled = !s.notify_enabled;
                                                cx.notify();
                                            });
                                        }),
                                    ))
                                    .child(header_chip(
                                        "btn-sound",
                                        if sound_on { "音ON" } else { "音OFF" },
                                        sound_on,
                                        cx.listener(|this, _, _, cx| {
                                            this.store.update(cx, |s, cx| {
                                                s.sound_enabled = !s.sound_enabled;
                                                if s.sound_enabled {
                                                    play_alert_sound(AlertSound::Done);
                                                }
                                                cx.notify();
                                            });
                                        }),
                                    ))
                                    .child(header_chip(
                                        "btn-sound-test",
                                        "♪",
                                        false,
                                        |_, _, _| {
                                            play_alert_sound(AlertSound::Done);
                                        },
                                    )),
                            ),
                    ),
            )
            // Summary KPI strip
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .bg(rgb(0x0d1526))
                    .border_b_1()
                    .border_color(rgb(0x1e293b))
                    .child(kpi_box("要対応", snap.waiting_count, rgb(0xf59e0b)))
                    .child(kpi_box("稼働中", snap.running_count, rgb(0x29b5e8)))
                    .child(kpi_box("スタック", snap.stalled_count, rgb(0xef4444)))
                    .child(kpi_box("新着完了", snap.unseen_completed_count, rgb(0x10b981))),
            )
            // Scrollable job list
            .child(
                v_flex()
                    .id("panel-scroll-body")
                    .flex_1()
                    .w_full()
                    .p_2p5()
                    .gap_2()
                    .overflow_y_scrollbar()
                    .when(active_jobs.is_empty() && unseen_done_jobs.is_empty(), |el| {
                        el.child(
                            v_flex()
                                .w_full()
                                .py_5()
                                .px_3()
                                .rounded_lg()
                                .bg(rgb(0x111827))
                                .border_1()
                                .border_color(rgb(0x1e293b))
                                .items_center()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(0x34d399))
                                        .child("稼働中・未確認の完了ジョブはありません"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(0x64748b))
                                        .child("完了ジョブはCoCoで開いて確認すると自動で消えます"),
                                ),
                        )
                    })
                    .children(
                        active_jobs
                            .iter()
                            .enumerate()
                            .map(|(i, j)| self.render_job_card(i, j, cx)),
                    )
                    .when(!unseen_done_jobs.is_empty(), |el| {
                        let show_done = self.show_completed;
                        let done_count = unseen_done_jobs.len();
                        el.child(
                            h_flex()
                                .w_full()
                                .mt_1()
                                .px_1()
                                .py_1()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .id("toggle-done-sec")
                                        .cursor_pointer()
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(rgb(0x6ee7b7))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.show_completed = !this.show_completed;
                                            cx.notify();
                                        }))
                                        .child(format!(
                                            "{} 新着完了・未確認 ({done_count})",
                                            if show_done { "▾" } else { "▸" }
                                        )),
                                )
                                .child(
                                    div()
                                        .id("clear-all-done")
                                        .px_2()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(rgb(0x1e293b))
                                        .hover(|s| s.bg(rgb(0x334155)))
                                        .cursor_pointer()
                                        .text_xs()
                                        .text_color(rgb(0xcbd5e1))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.store.update(cx, |s, cx| {
                                                s.dismiss_all_done();
                                                cx.notify();
                                            });
                                        }))
                                        .child("一括クリア"),
                                ),
                        )
                        .when(show_done, |el2| {
                            el2.children(
                                unseen_done_jobs
                                    .iter()
                                    .enumerate()
                                    .map(|(i, j)| self.render_job_card(1000 + i, j, cx)),
                            )
                        })
                    }),
            )
            // Bottom credit footer
            .when_some(
                snap.daily_credits_used.zip(snap.daily_credits_limit),
                |el, (used, limit)| {
                    el.child(
                        h_flex()
                            .w_full()
                            .px_3()
                            .py_1p5()
                            .bg(rgb(0x090d16))
                            .border_t_1()
                            .border_color(rgb(0x1e293b))
                            .items_center()
                            .justify_between()
                            .text_xs()
                            .text_color(rgb(0x64748b))
                            .child("Hooks + FSEvents 連動中 (127.0.0.1)")
                            .child(format!("Credits {:.1} / {:.0}", used, limit)),
                    )
                },
            )
            .into_any_element()
    }
}

fn header_chip(
    id: &'static str,
    label: &'static str,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_2()
        .py_0p5()
        .rounded_md()
        .bg(if active { rgb(0x172554) } else { rgb(0x1e293b) })
        .border_1()
        .border_color(if active { rgb(0x2563eb) } else { rgb(0x334155) })
        .hover(|s| s.border_color(rgb(0x38bdf8)))
        .cursor_pointer()
        .text_xs()
        .text_color(if active { rgb(0x93c5fd) } else { rgb(0x94a3b8) })
        .on_click(on_click)
        .child(label)
}

fn kpi_box(label: &'static str, count: usize, accent: Rgba) -> impl IntoElement {
    v_flex()
        .flex_1()
        .px_2()
        .py_1()
        .rounded_md()
        .bg(rgb(0x111827))
        .border_1()
        .border_color(if count > 0 { accent } else { rgb(0x1e293b) })
        .items_center()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .text_color(if count > 0 { accent } else { rgb(0x64748b) })
                .child(count.to_string()),
        )
        .child(div().text_xs().text_color(rgb(0x94a3b8)).child(label))
}
