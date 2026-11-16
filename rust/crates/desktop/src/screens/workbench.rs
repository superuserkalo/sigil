//! Workbench: transcript, prompt composer, provider status and the running tool timeline.

use rune::tok::motion;
use rune::{Color, El, ScrollSpec, col, el, row, spacer, text};

use crate::agent::{AgentStatus, Pause};
use crate::app::State;
use crate::model::{Message, Role, RunState, Screen, ToolRun};
use crate::ui::{ActionKind, BODY, BODY_MED, LABEL, MONO, MONO_MED, RAIL_W, SMALL, Tone, Ui};

pub fn view(s: &State, u: &Ui) -> El {
    let project = s.project();
    let title = match project {
        Some(p) => format!("{} session", p.name),
        None => "General workbench".to_string(),
    };
    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(u.status_chip(
            &format!("host {}", s.agent.status.label()),
            status_tone(s.agent.status),
        ))
        .child(
            u.action("New session", ActionKind::Secondary)
                .on_press(|st: &mut State| st.begin_session()),
        );
    let detail = match project {
        Some(p) => format!(
            "One conversation for {}. Tools stage and validate; only the app writes, behind approval.",
            p.name
        ),
        None => "One conversation per mod project. Open a project for scoped paths, or ask a general question."
            .to_string(),
    };

    let mut column = col()
        .key("feed-column")
        .grow(1.0)
        .h_full()
        .min_w(0.0)
        .min_h(0.0)
        .gap(12.0)
        .child(host_card(s, u));
    if let Some(error) = &s.agent.error {
        column = column.child(host_error_card(s, u, error));
    }
    column = column.child(feed(s, u));
    if let Some(pause) = &s.agent.pause {
        column = column.child(pause_card(u, pause));
    }
    column = column.child(composer(s, u));

    col()
        .key("workbench-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(20.0)
        .pb(16.0)
        .gap(14.0)
        .child(u.page_head(Screen::Workbench.kicker(), &title, &detail, Some(trailing)))
        .child(
            row()
                .key("workbench-body")
                .grow(1.0)
                .min_h(0.0)
                .gap(16.0)
                .child(column)
                .child(rail(s, u)),
        )
}

fn status_tone(status: AgentStatus) -> Tone {
    match status {
        AgentStatus::Ready => Tone::Ok,
        AgentStatus::Running => Tone::Warn,
        AgentStatus::Paused => Tone::Warn,
        AgentStatus::Starting => Tone::Info,
        AgentStatus::Error => Tone::Bad,
        AgentStatus::Stopped => Tone::Quiet,
    }
}

fn host_card(s: &State, u: &Ui) -> El {
    let tone = status_tone(s.agent.status);
    row()
        .key("host-card")
        .w_full()
        .min_h(38.0)
        .px(11.0)
        .py(7.0)
        .gap(9.0)
        .center_y()
        .radius(8.0)
        .fill(Color::WHITE.alpha(0.022))
        .border(1.0, u.hair)
        .child(u.dot(tone))
        .child(text("HOST").font(LABEL).color(u.faint))
        .child(u.status_chip(s.agent.status.label(), tone))
        .child(
            text(s.agent.binary_label())
                .font(MONO)
                .color(u.muted)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
        .child(u.chip(
            &s.agent.provider_label(),
            if s.agent.demo || s.agent.configured {
                u.teal
            } else {
                u.rose
            },
            if s.agent.demo || s.agent.configured {
                u.teal.alpha(0.10)
            } else {
                u.rose.alpha(0.10)
            },
        ))
        .child(
            text(format!("session {}", s.agent.session_label()))
                .font(MONO)
                .color(u.faint),
        )
}

fn host_error_card(s: &State, u: &Ui, error: &str) -> El {
    let mut card = col()
        .key("host-error")
        .w_full()
        .gap(6.0)
        .padding(10.0)
        .radius(8.0)
        .fill(u.rose.alpha(0.08))
        .border(1.0, u.rose.alpha(0.28))
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(u.status_chip("HOST ERROR", Tone::Bad))
                .child(
                    text(if s.agent.configured {
                        "The host is not answering."
                    } else {
                        "The host is not configured."
                    })
                    .font(SMALL)
                    .color(u.muted),
                )
                .child(spacer())
                .child(
                    u.action("Restart host", ActionKind::Ghost)
                        .on_press(|st: &mut State| st.restart_host()),
                ),
        )
        .child(text(error).font(SMALL).color(u.ink).wrap().selectable(true));
    if !s.agent.configured {
        card = card.child(
            text("Required: DAH_AGENT_MODEL. Optional: DAH_AGENT_PROVIDER (openai, anthropic, chat), DAH_AGENT_BASE_URL, DAH_AGENT_API_KEY.")
                .font(MONO)
                .color(u.faint)
                .wrap_anywhere(),
        );
    }
    card
}

fn pause_card(u: &Ui, pause: &Pause) -> El {
    u.panel()
        .key("approval-card")
        .w_full()
        .border(1.0, u.amber.alpha(0.34))
        .child(
            row()
                .w_full()
                .px(12.0)
                .pt(10.0)
                .pb(8.0)
                .gap(8.0)
                .center_y()
                .child(u.dot(Tone::Warn))
                .child(text("APPROVAL REQUIRED").font(LABEL).color(u.amber))
                .child(u.chip(&pause.tool_name, u.amber, u.amber.alpha(0.12)))
                .child(spacer())
                .child(
                    text(format!("call {}", pause.tool_call_id))
                        .font(MONO)
                        .color(u.faint),
                ),
        )
        .child(
            col()
                .w_full()
                .px(12.0)
                .pb(10.0)
                .gap(7.0)
                .child(u.field_label("PAYLOAD"))
                .child(
                    text(pause.payload_text())
                        .font(MONO)
                        .color(u.prose)
                        .wrap_anywhere()
                        .selectable(true),
                ),
        )
        .child(
            row()
                .w_full()
                .px(12.0)
                .pb(11.0)
                .gap(8.0)
                .center_y()
                .child(
                    u.action("Approve", ActionKind::Primary)
                        .key("approve")
                        .on_press(|st: &mut State| st.approve_pause()),
                )
                .child(
                    u.action("Deny", ActionKind::Secondary)
                        .key("deny")
                        .on_press(|st: &mut State| st.deny_pause()),
                )
                .child(spacer())
                .child(
                    text("Approve sends resume with a result; Deny sends resume with an error.")
                        .font(SMALL)
                        .color(u.faint),
                ),
        )
}

fn feed(s: &State, u: &Ui) -> El {
    let mut list = col().key("feed").w_full().gap(18.0).pb(4.0);
    if !s.demo && s.messages.len() <= 1 {
        list = list.child(start_strip(u));
    }
    for (i, m) in s.messages.iter().enumerate() {
        list = list.child(message(s, u, i, m));
    }
    col()
        .key("transcript")
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: true })
        .child(list)
}

fn start_strip(u: &Ui) -> El {
    row()
        .key("start-strip")
        .w_full()
        .gap(10.0)
        .child(start_card(
            u,
            "▤",
            "Open a project",
            "Give the agent a manifest, slot and target list.",
            |st: &mut State| st.screen = Screen::Projects,
        ))
        .child(start_card(
            u,
            "⌕",
            "Build the game index",
            "Find exact paths before staging anything.",
            |st: &mut State| {
                st.screen = Screen::Index;
                st.status = "Index build is a CLI action: run `pnpm dah index` after setting the game path."
                    .to_string();
            },
        ))
        .child(start_card(
            u,
            "⚙",
            "Configure a provider",
            "BYOK keys stay in the OS store, never in this window.",
            |st: &mut State| st.screen = Screen::Settings,
        ))
}

fn start_card(u: &Ui, glyph: &str, title: &str, detail: &str, press: fn(&mut State)) -> El {
    col()
        .grow(1.0)
        .gap(6.0)
        .padding(12.0)
        .radius(8.0)
        .fill(Color::WHITE.alpha(0.024))
        .border(1.0, u.hair)
        .hover(Color::WHITE.alpha(0.045))
        .press(Color::WHITE.alpha(0.06))
        .focusable(true)
        .role("button")
        .label(title)
        .motion(motion::SNAPPY)
        .on_press(press)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(u.glyph_box(glyph, Tone::Quiet, 24.0))
                .child(text(title).font(BODY_MED).color(u.ink)),
        )
        .child(text(detail).font(SMALL).color(u.faint).wrap())
}

fn message(s: &State, u: &Ui, idx: usize, m: &Message) -> El {
    match m.role {
        Role::User => user_row(u, idx, m),
        Role::Assistant => assistant_row(s, u, idx, m),
        Role::Tool => tool_row(s, u, idx, m.tool.as_ref()),
    }
}

fn user_row(u: &Ui, idx: usize, m: &Message) -> El {
    row()
        .key(format!("msg-{idx}"))
        .w_full()
        .gap(12.0)
        .items_start()
        .child(avatar(u, "YOU", Tone::Quiet))
        .child(
            col()
                .grow(1.0)
                .gap(5.0)
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(text("You").font(BODY_MED).color(u.ink))
                        .child(text(m.meta.clone()).font(SMALL).color(u.faint)),
                )
                .child(
                    text(m.body.clone())
                        .font(BODY)
                        .color(u.ink)
                        .wrap()
                        .selectable(true),
                ),
        )
}

fn assistant_row(s: &State, u: &Ui, idx: usize, m: &Message) -> El {
    let mut meta = row()
        .gap(8.0)
        .center_y()
        .child(text("Workbench").font(BODY_MED).color(u.ink));
    if let Some(model) = &m.model {
        meta = meta.child(u.chip(model, u.teal, u.teal.alpha(0.10)));
    }
    meta = meta.child(text(m.meta.clone()).font(SMALL).color(u.faint));
    let mut body = col().grow(1.0).gap(5.0).child(meta).child(
        text(m.body.clone())
            .font(BODY)
            .color(u.prose)
            .wrap()
            .selectable(true),
    );
    if s.tools.iter().any(|t| t.state == RunState::Running) && idx + 1 == s.messages.len() {
        body = body.child(
            row()
                .gap(7.0)
                .center_y()
                .child(pulse_dot(u, s.pulse))
                .child(text("Working").font(SMALL).color(u.amber)),
        );
    }
    row()
        .key(format!("msg-{idx}"))
        .w_full()
        .gap(12.0)
        .items_start()
        .child(avatar(u, "◆", Tone::Accent))
        .child(body)
}

fn tool_row(_s: &State, u: &Ui, idx: usize, tool: Option<&ToolRun>) -> El {
    let Some(tool) = tool else {
        return el().key(format!("msg-{idx}")).h(0.0);
    };
    let tone = run_tone(tool.state);
    let mut head = row()
        .px(11.0)
        .pt(5.0)
        .gap(8.0)
        .center_y()
        .child(text("▸").font(MONO).color(tone.fg(u)).a11y_hidden())
        .child(text(tool.name.clone()).font(MONO_MED).color(u.ink))
        .child(u.status_chip(tool.state.label(), tone))
        .child(spacer());
    if !tool.duration.is_empty() {
        head = head.child(
            text(tool.duration.clone())
                .font(SMALL)
                .color(u.faint)
                .w(46.0)
                .align(rune::Align::End),
        );
    }
    let mut card = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(5.0)
        .py(6.0)
        .radius(8.0)
        .fill(Color::WHITE.alpha(0.025))
        .border(1.0, u.hair)
        .child(head);
    if !tool.args.is_empty() {
        card = card.child(
            text(tool.args.clone())
                .font(MONO)
                .color(u.faint)
                .px(11.0)
                .wrap_anywhere(),
        );
    }
    card = card.child(
        row()
            .px(11.0)
            .pb(5.0)
            .gap(7.0)
            .items_center()
            .child(u.dot(tone))
            .child(
                text(tool.result.clone())
                    .font(SMALL)
                    .color(u.muted)
                    .grow(1.0)
                    .wrap(),
            ),
    );
    row()
        .key(format!("msg-{idx}"))
        .w_full()
        .gap(12.0)
        .items_start()
        .child(el().w(26.0).h(1.0))
        .child(card)
}

fn avatar(u: &Ui, glyph: &str, tone: Tone) -> El {
    el().size(26.0, 26.0)
        .radius(7.0)
        .center()
        .fill(tone.bg(u))
        .border(1.0, tone.fg(u).alpha(0.22))
        .child(text(glyph).font(LABEL).color(tone.fg(u)).a11y_hidden())
        .a11y_hidden()
}

fn run_tone(state: RunState) -> Tone {
    match state {
        RunState::Done => Tone::Ok,
        RunState::Running => Tone::Warn,
        RunState::Queued => Tone::Quiet,
    }
}

fn pulse_dot(u: &Ui, pulse: f32) -> El {
    el().size(7.0, 7.0)
        .radius(4.0)
        .fill(u.amber.alpha(0.35 + 0.65 * pulse))
        .a11y_hidden()
}

fn composer(s: &State, u: &Ui) -> El {
    let field = rune::text_input(&s.composer, BODY)
        .key("composer")
        .label("Prompt")
        .grow(1.0)
        .min_h(22.0)
        .max_h(96.0)
        .color(u.ink)
        .placeholder("Ask the workbench, or describe the mod you want", u.faint)
        .caret(u.accent, u.accent.alpha(0.22))
        .on_edit(|st: &mut State| &mut st.composer)
        .on_submit(|st: &mut State, _| st.send_prompt());

    let ready = !s.composer.text.trim().is_empty() && s.agent.status == AgentStatus::Ready;
    let running = s.agent.status == AgentStatus::Running;
    let action = if running {
        u.action("Stop", ActionKind::Secondary)
            .key("abort")
            .on_press(|st: &mut State| st.abort_run())
    } else {
        let mut send = u
            .action("Send", ActionKind::Primary)
            .key("send")
            .on_press(|st: &mut State| st.send_prompt());
        if !ready {
            send = send.opacity(0.55);
        }
        send
    };

    col()
        .key("composer-card")
        .w_full()
        .radius(11.0)
        .fill(u.panel)
        .border(1.0, u.hair_strong)
        .shadow(10.0, 28.0, u.shadow)
        .child(
            row()
                .w_full()
                .px(13.0)
                .pt(11.0)
                .pb(4.0)
                .gap(10.0)
                .items_end()
                .child(field)
                .child(action),
        )
        .child(
            row()
                .w_full()
                .px(12.0)
                .pb(9.0)
                .gap(8.0)
                .center_y()
                .child(u.dot(status_tone(s.agent.status)))
                .child(
                    text(if s.agent.demo {
                        format!(
                            "{}. Demo fixture, no host process.",
                            s.agent.provider_label()
                        )
                    } else if s.agent.configured {
                        format!(
                            "{} · host {}",
                            s.agent.provider_label(),
                            s.agent.status.label()
                        )
                    } else {
                        "Agent host not configured".to_string()
                    })
                    .font(SMALL)
                    .color(u.faint),
                )
                .child(text("·").font(SMALL).color(u.faint).a11y_hidden())
                .child(text("Enter sends").font(SMALL).color(u.faint))
                .child(spacer())
                .child(
                    text("Tools never write to the game install")
                        .font(SMALL)
                        .color(u.faint),
                ),
        )
}

fn rail(s: &State, u: &Ui) -> El {
    let tone = status_tone(s.agent.status);
    let provider_card = u
        .panel()
        .key("provider-card")
        .padding(12.0)
        .gap(8.0)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(text("MODEL").font(LABEL).color(u.faint))
                .child(spacer())
                .child(u.dot(tone))
                .child(u.status_chip(s.agent.status.label(), tone)),
        )
        .child(
            text(if s.agent.demo {
                "Demo fixture: no host process".to_string()
            } else {
                s.agent.provider_label()
            })
            .font(MONO)
            .color(if s.agent.configured || s.agent.demo {
                u.teal
            } else {
                u.rose
            })
            .truncate(),
        )
        .child(
            row()
                .gap(8.0)
                .child(text("usage").font(SMALL).color(u.faint))
                .child(spacer())
                .child(
                    text(s.agent.usage_label())
                        .font(SMALL)
                        .color(u.muted)
                        .truncate(),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(text("session").font(SMALL).color(u.faint))
                .child(spacer())
                .child(
                    text(s.agent.session_label())
                        .font(MONO)
                        .color(u.muted)
                        .truncate(),
                ),
        )
        .child(
            text(if s.agent.demo {
                "Demo scene. No host process and no model call."
            } else {
                "Provider keys are read by the host. This window only sees status."
            })
            .font(SMALL)
            .color(u.faint)
            .wrap(),
        );

    let mut timeline = col().key("timeline-list").w_full().gap(0.0);
    if s.tools.is_empty() {
        timeline = timeline.padding(12.0).child(
            text("No tools have run in this session.")
                .font(SMALL)
                .color(u.faint)
                .wrap(),
        );
    } else {
        for (i, run) in s.tools.iter().enumerate() {
            if i > 0 {
                timeline = timeline.child(u.divider());
            }
            timeline = timeline.child(timeline_row(s, u, run));
        }
    }

    let timeline_card = u
        .panel()
        .key("timeline-card")
        .grow(if s.tools.is_empty() { 0.0 } else { 1.0 })
        .min_h(0.0)
        .child(u.panel_header("TOOL TIMELINE", &format!("{} calls", s.tools.len())))
        .child(
            col()
                .grow(if s.tools.is_empty() { 0.0 } else { 1.0 })
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(timeline),
        );

    let boundary_card = u
        .panel()
        .key("boundary-card")
        .padding(12.0)
        .gap(7.0)
        .child(text("WRITE BOUNDARY").font(LABEL).color(u.faint))
        .child(boundary_row(u, "Staging folder", "agent", Tone::Ok))
        .child(boundary_row(u, "Game install", "approval", Tone::Warn))
        .child(boundary_row(u, "Publish", "approval", Tone::Warn));

    let session_card = u
        .panel()
        .key("session-card")
        .padding(12.0)
        .gap(10.0)
        .child(text("SESSION").font(LABEL).color(u.faint))
        .child(
            row()
                .gap(8.0)
                .child(u.stat(format!("{}", s.messages.len()), "MESSAGES", Tone::Info))
                .child(u.stat(format!("{}", s.tools.len()), "TOOLS", Tone::Accent))
                .child(u.stat(s.elapsed.clone(), "ELAPSED", Tone::Quiet)),
        );

    let bottom_pad = if s.tools.is_empty() {
        spacer()
    } else {
        el().h(0.0)
    };

    col()
        .key("rail")
        .w(RAIL_W)
        .h_full()
        .min_h(0.0)
        .pl(16.0)
        .gap(12.0)
        .border_sides([0.0, 0.0, 0.0, 1.0], u.hair)
        .child(provider_card)
        .child(timeline_card)
        .child(boundary_card)
        .child(session_card)
        .child(bottom_pad)
        .child(
            text("The running tool drives this timeline. No tool writes to the game install.")
                .font(SMALL)
                .color(u.faint)
                .wrap(),
        )
}

fn boundary_row(u: &Ui, label: &str, state: &str, tone: Tone) -> El {
    row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(text(label).font(SMALL).color(u.muted).grow(1.0))
        .child(u.chip(state, tone.fg(u), tone.bg(u)))
}

fn timeline_row(s: &State, u: &Ui, run: &ToolRun) -> El {
    let tone = run_tone(run.state);
    let dot = if run.state == RunState::Running {
        pulse_dot(u, s.pulse)
    } else {
        u.dot(tone)
    };
    let mut r = row()
        .w_full()
        .h(35.0)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .child(dot)
        .child(
            text(run.name.clone())
                .font(MONO)
                .color(u.ink)
                .grow(1.0)
                .truncate(),
        )
        .child(text(run.state.label()).font(LABEL).color(tone.fg(u)));
    if !run.duration.is_empty() {
        r = r.child(
            text(run.duration.clone())
                .font(SMALL)
                .color(u.faint)
                .w(38.0)
                .align(rune::Align::End),
        );
    }
    r
}
