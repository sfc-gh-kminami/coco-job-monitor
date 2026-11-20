use crate::store::{JobStore, focus_coco_workspace, open_chrome_dashboard};
use crate::types::{JobItem, JobStatus, LOOPBACK_PORT, format_duration_ja};
use gpui_kit::component::button::Button;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::BTreeSet;

pub struct BoardView {
    store: Entity<JobStore>,
    selected_workspace: Option<String>,
    show_archived_done: bool,
}

impl BoardView {
    pub fn new(store: Entity<JobStore>, cx: &mut Context<Self>) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        Self {
            store,
            selected_workspace: None,
            show_archived_done: false,
        }
    }

    fn render_board_card(&self, idx: usize, job: &JobItem) -> AnyElement {
        let (accent, card_bg, badge_bg, badge_fg) = match job.status {
            JobStatus::WaitingPermission | JobStatus::WaitingInput => {
                (rgb(0xf59e0b), rgb(0x231c12), rgb(0x78350f), rgb(0xfde68a))
            }
            JobStatus::Running => (rgb(0x29b5e8), rgb(0x141e33), rgb(0x0c4a6e), rgb(0x7dd3fc)),
            JobStatus::Stalled => (rgb(0xef4444), rgb(0x26151a), rgb(0x7f1d1d), rgb(0xfecaca)),
            JobStatus::Error => (rgb(0xef4444), rgb(0x1f151a), rgb(0x7f1d1d), rgb(0xfca5a5)),
            JobStatus::Completed => (rgb(0x10b981), rgb(0x131b2b), rgb(0x064e3b), rgb(0x6ee7b7)),
        };

        let cwd = job.working_directory.clone();
        let ctx_ratio = if job.context_window > 0 {
            (job.context_tokens as f32 / job.context_window as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };

        v_flex()
            .id(("board-card", idx))
            .w_full()
            .p_3()
            .gap_2()
            .rounded_xl()
            .bg(card_bg)
            .border_1()
            .border_color(if job.status.is_attention() || job.status == JobStatus::Stalled {
                accent
            } else {
                rgb(0x26344d)
            })
            .hover(|s| s.border_color(rgb(0x38bdf8)))
            .cursor_pointer()
            .on_click(move |_, _, _| {
                focus_coco_workspace(&cwd);
            })
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1p5()
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_md()
                                    .bg(badge_bg)
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(badge_fg)
                                    .child(job.status.label()),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_md()
                                    .bg(rgb(0x0b1120))
                                    .text_xs()
                                    .text_color(rgb(0x93c5fd))
                                    .child(job.workspace_name.clone()),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if job.status == JobStatus::Running && job.heartbeat_secs <= 15 {
                                rgb(0x34d399)
                            } else {
                                rgb(0x94a3b8)
                            })
                            .child(format!(
                                "最終動作 {}前 · 経過 {}",
                                format_duration_ja(job.heartbeat_secs),
                                format_duration_ja(job.elapsed_secs)
                            )),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(0xf8fafc))
                    .child(job.title.clone()),
            )
            .when(job.status != JobStatus::Completed, |el| {
                el.child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .when(job.status == JobStatus::Running, |r| {
                            r.child(Spinner::new().xsmall().color(accent.into()))
                        })
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(0xe2e8f0))
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
                        .p_2()
                        .rounded_lg()
                        .bg(rgb(0x0d1424))
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .text_xs()
                                .text_color(rgb(0xcbd5e1))
                                .child("Todo 進捗")
                                .child(format!(
                                    "{} / {} ({}%)",
                                    todo.completed,
                                    todo.total,
                                    (ratio * 100.0).round() as usize
                                )),
                        )
                        .child(
                            div()
                                .w_full()
                                .h(px(6.))
                                .rounded_full()
                                .bg(rgb(0x070b14))
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
                        .when_some(todo.current_task, |c, task| {
                            c.child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x94a3b8))
                                    .child(format!("進行中: {task}")),
                            )
                        }),
                )
            })
            .when(!job.subagents.is_empty(), |el| {
                el.child(
                    v_flex()
                        .w_full()
                        .p_2()
                        .rounded_lg()
                        .bg(rgb(0x0d1424))
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(0x93c5fd))
                                .child(format!("サブエージェント ({})", job.subagents.len())),
                        )
                        .children(job.subagents.iter().take(6).map(|sub| {
                            h_flex()
                                .w_full()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .text_xs()
                                .child(
                                    div()
                                        .text_color(if sub.active {
                                            rgb(0x7dd3fc)
                                        } else {
                                            rgb(0x64748b)
                                        })
                                        .truncate()
                                        .child(format!(
                                            "{} {}",
                                            if sub.active { "●" } else { "✓" },
                                            sub.title
                                        )),
                                )
                                .child(
                                    div()
                                        .text_color(rgb(0x94a3b8))
                                        .flex_shrink_0()
                                        .child(sub.current_step.clone()),
                                )
                        })),
                )
            })
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .pt_1()
                    .text_xs()
                    .text_color(rgb(0x64748b))
                    .child(format!("Model: {}", job.model))
                    .child({
                        let mut meta = Vec::new();
                        if job.context_window > 0 {
                            meta.push(format!(
                                "Ctx {}k/{}k ({}%)",
                                job.context_tokens / 1000,
                                job.context_window / 1000,
                                (ctx_ratio * 100.0).round() as usize
                            ));
                        }
                        if job.diff_files > 0 {
                            meta.push(format!(
                                "+{} -{} ({} files)",
                                job.diff_added, job.diff_removed, job.diff_files
                            ));
                        }
                        meta.join(" · ")
                    }),
            )
            .into_any_element()
    }

    fn render_lane(
        &self,
        lane_id: &'static str,
        title: &'static str,
        accent: Rgba,
        items: &[JobItem],
        base_idx: usize,
    ) -> impl IntoElement {
        v_flex()
            .flex_1()
            .h_full()
            .rounded_xl()
            .bg(rgb(0x0f172a))
            .border_1()
            .border_color(rgb(0x1e293b))
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .px_3p5()
                    .py_2p5()
                    .bg(rgb(0x111c33))
                    .border_b_1()
                    .border_color(rgb(0x1e293b))
                    .items_center()
                    .justify_between()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(div().w(px(8.)).h(px(8.)).rounded_full().bg(accent))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(0xe2e8f0))
                                    .child(title),
                            ),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded_full()
                            .bg(rgb(0x0b1120))
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(accent)
                            .child(items.len().to_string()),
                    ),
            )
            .child(
                v_flex()
                    .id(lane_id)
                    .flex_1()
                    .w_full()
                    .p_3()
                    .gap_2p5()
                    .overflow_y_scrollbar()
                    .when(items.is_empty(), |el| {
                        el.child(
                            div()
                                .py_6()
                                .text_center()
                                .text_xs()
                                .text_color(rgb(0x64748b))
                                .child("該当するジョブはありません"),
                        )
                    })
                    .children(
                        items
                            .iter()
                            .enumerate()
                            .map(|(i, j)| self.render_board_card(base_idx + i, j)),
                    ),
            )
    }
}

impl Render for BoardView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snap = self.store.read(cx).snapshot.clone();
        let workspaces: Vec<String> = snap
            .jobs
            .iter()
            .map(|j| j.workspace_name.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        let filtered: Vec<JobItem> = snap
            .jobs
            .iter()
            .filter(|j| {
                self.selected_workspace
                    .as_ref()
                    .map(|ws| &j.workspace_name == ws)
                    .unwrap_or(true)
            })
            .cloned()
            .collect();

        let mut attention_lane = Vec::new();
        let mut running_lane = Vec::new();
        let mut completed_lane = Vec::new();

        for j in filtered {
            if j.status.is_attention() || j.status == JobStatus::Stalled {
                attention_lane.push(j);
            } else if j.status == JobStatus::Running {
                running_lane.push(j);
            } else if j.unseen_done || self.show_archived_done {
                completed_lane.push(j);
            }
        }

        v_flex()
            .size_full()
            .bg(rgb(0x0b1120))
            .text_color(rgb(0xf8fafc))
            .p_4()
            .gap_3p5()
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .pb_3()
                    .border_b_1()
                    .border_color(rgb(0x1e293b))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .px_2p5()
                                    .py_1()
                                    .rounded_md()
                                    .bg(rgb(0x0c4a6e))
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(0x38bdf8))
                                    .child("COCO JOB BOARD"),
                            )
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::BOLD)
                                    .child("全ワークスペース横断 Job 管理ボード (GPUI Native)"),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .when_some(
                                snap.daily_credits_used.zip(snap.daily_credits_limit),
                                |el, (used, limit)| {
                                    el.child(
                                        div()
                                            .px_3()
                                            .py_1()
                                            .rounded_lg()
                                            .bg(rgb(0x111827))
                                            .border_1()
                                            .border_color(rgb(0x1e293b))
                                            .text_xs()
                                            .text_color(rgb(0x94a3b8))
                                            .child(format!(
                                                "本日推定クレジット: {:.1} / {:.0}",
                                                used, limit
                                            )),
                                    )
                                },
                            )
                            .child(
                                Button::new("toggle-archived-done")
                                    .small()
                                    .outline()
                                    .label(if self.show_archived_done {
                                        "未確認の完了のみ表示"
                                    } else {
                                        "確認済み履歴も表示"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_archived_done = !this.show_archived_done;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("open-in-chrome-board")
                                    .small()
                                    .outline()
                                    .label("Chromeでも開く")
                                    .on_click(|_, _, _| {
                                        open_chrome_dashboard(LOOPBACK_PORT);
                                    }),
                            ),
                    ),
            )
            // Workspace filter strip
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_1p5()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x94a3b8))
                            .mr_1()
                            .child("プロジェクト絞り込み:"),
                    )
                    .child({
                        let is_all = self.selected_workspace.is_none();
                        div()
                            .id("ws-filter-all")
                            .px_2p5()
                            .py_1()
                            .rounded_md()
                            .bg(if is_all { rgb(0x1d4ed8) } else { rgb(0x111827) })
                            .text_xs()
                            .text_color(if is_all { rgb(0xffffff) } else { rgb(0x94a3b8) })
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.selected_workspace = None;
                                cx.notify();
                            }))
                            .child("すべて")
                    })
                    .children(workspaces.into_iter().enumerate().map(|(i, ws)| {
                        let active = self.selected_workspace.as_ref() == Some(&ws);
                        let ws_click = ws.clone();
                        div()
                            .id(("ws-filter", i))
                            .px_2p5()
                            .py_1()
                            .rounded_md()
                            .bg(if active { rgb(0x1d4ed8) } else { rgb(0x111827) })
                            .text_xs()
                            .text_color(if active { rgb(0xffffff) } else { rgb(0x94a3b8) })
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.selected_workspace = Some(ws_click.clone());
                                cx.notify();
                            }))
                            .child(ws)
                    })),
            )
            // 3 Kanban lanes
            .child(
                h_flex()
                    .flex_1()
                    .w_full()
                    .gap_3p5()
                    .overflow_hidden()
                    .child(self.render_lane(
                        "lane-attention",
                        "要対応（承認・回答待ち / スタック疑い）",
                        rgb(0xf59e0b),
                        &attention_lane,
                        0,
                    ))
                    .child(self.render_lane(
                        "lane-running",
                        "稼働中（思考・ツール・サブエージェント）",
                        rgb(0x29b5e8),
                        &running_lane,
                        1000,
                    ))
                    .child(self.render_lane(
                        "lane-completed",
                        if self.show_archived_done {
                            "完了・エラー履歴（確認済み含む）"
                        } else {
                            "新着完了・未確認（確認後は自動で外れます）"
                        },
                        rgb(0x10b981),
                        &completed_lane,
                        2000,
                    )),
            )
    }
}
