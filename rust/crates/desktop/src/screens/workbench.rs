//! Workbench: one conversation canvas, a quiet context rail and the prompt
//! composer anchored at the bottom. Tool calls stay compact; approvals appear
//! inline, in the conversation, where the run paused.

use rune::tok::motion;
use rune::{Color, El, ScrollSpec, col, el, row, spacer, text};

use crate::agent::{AgentStatus, Pause};
use crate::app::State;
use crate::model::{Message, Role, RunState, Screen, ToolRun};
use crate::ui::{
    ActionKind, BODY_MED, CHAT, HERO, LABEL, LEAD, MONO, MONO_MED, RAIL_W, SMALL, Tone, Ui,
};

pub fn view(s: &State, u: &Ui) -> El {
    let project = s.project();
    let title = match project {
        Some(p) => p.name.clone(),
        None => "General session".to_string(),
    };
    let detail = match project {
        Some(p) => format!(
            "A working conversation for {}. Tools stage and validate, and only the app writes, behind your approval.",
            p.name
        ),
        None => {
            "Open a project for scoped paths, or ask a general question about Deadlock modding."
                .to_string()
        }
    };
    let trailing = row()
        .gap(9.0)
        .center_y()
        .child(u.status_chip(
            &format!("host {}", s.agent.status.label()),
            status_tone(s.agent.status),
        ))
        .child(
            u.pill("New session", "＋", ActionKind::Ghost)
                .key("new-session")
                .on_press(|st: &mut State| st.begin_session()),
        );

    let mut column = col()
        .key("feed-column")
        .grow(1.0)
        .h_full()
        .min_w(0.0)
        .min_h(0.0)
        .gap(13.0)
        .child(session_strip(s, u));
    if let Some(error) = &s.agent.error {
        column = column.child(host_error_card(s, u, error));
    }
    column = column.child(feed(s, u));
    if let Some(pause) = &s.agent.pause {
        column = column.child(pause_card(s, u, pause));
    }
    column = column.child(composer(s, u));

    col()
        .key("workbench-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(26.0)
        .pt(20.0)
        .pb(16.0)
        .gap(16.0)
        .child(u.glow(
            "workbench-glow-warm",
            [0.20, 0.0],
            (700.0, 480.0),
            u.glow_warm,
        ))
        .child(u.glow(
            "workbench-glow-cool",
            [0.88, 0.10],
            (640.0, 520.0),
            u.glow_cool,
        ))
        .child(head(u, &title, &detail, trailing))
        .child(
            row()
                .key("workbench-body")
                .grow(1.0)
                .min_h(0.0)
                .gap(18.0)
                .child(column)
                .child(rail(s, u)),
        )
}

fn head(u: &Ui, title: &str, detail: &str, trailing: El) -> El {
    row()
        .w_full()
        .gap(20.0)
        .items_end()
        .child(
            col()
                .grow(1.0)
                .gap(7.0)
                .max_w(760.0)
                .child(u.eyebrow(Screen::Agent.kicker()))
                .child(text(title).font(HERO).color(u.ink))
                .child(text(detail).font(LEAD).color(u.muted).wrap()),
        )
        .child(trailing)
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

fn session_strip(s: &State, u: &Ui) -> El {
    let tone = status_tone(s.agent.status);
    let mut strip = row()
        .key("session-strip")
        .w_full()
        .min_h(34.0)
        .px(12.0)
        .py(6.0)
        .gap(9.0)
        .center_y()
        .child(u.dot(tone))
        .child(text("SESSION").font(LABEL).color(u.faint))
        .child(u.status_chip(s.agent.status.label(), tone));
    if s.publish_job.is_running()
        && let Some(label) = s.publish_job.label()
    {
        strip = strip.child(u.chip(&format!("{label} running"), u.amber, u.amber.alpha(0.10)));
    }
    strip = strip
        .child(
            text(if s.agent.demo {
                "Demo fixture, no host process".to_string()
            } else {
                s.agent.provider_label()
            })
            .font(MONO)
            .color(if s.agent.demo || s.agent.configured {
                u.muted
            } else {
                u.rose
            })
            .grow(1.0)
            .min_w(0.0)
            .truncate(),
        )
        .child(
            text(format!("session {}", s.agent.session_label()))
                .font(MONO)
                .color(u.faint)
                .truncate(),
        );
    u.glass_shell()
        .w_full()
        .padding(1.0)
        .child(u.glass_core().w_full().child(strip))
}

fn host_error_card(s: &State, u: &Ui, error: &str) -> El {
    let mut card = col()
        .key("host-error")
        .w_full()
        .gap(7.0)
        .padding(14.0)
        .radius(14.0)
        .fill(u.rose.alpha(0.055))
        .border(1.0, u.rose.alpha(0.20))
        .child(
            row()
                .gap(9.0)
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

fn pause_card(s: &State, u: &Ui, pause: &Pause) -> El {
    let inner = col()
        .w_full()
        .padding(18.0)
        .gap(13.0)
        .child(
            row()
                .w_full()
                .gap(10.0)
                .center_y()
                .child(u.glyph_box("!", Tone::Warn, 30.0))
                .child(
                    col()
                        .grow(1.0)
                        .gap(2.0)
                        .child(text("APPROVAL REQUIRED").font(LABEL).color(u.amber))
                        .child(
                            text(format!("{} is paused before it runs", pause.tool_name))
                                .font(BODY_MED)
                                .color(u.ink),
                        ),
                )
                .child(u.chip(&pause.tool_name, u.amber, u.amber.alpha(0.12)))
                .child(
                    text(format!("call {}", pause.tool_call_id))
                        .font(MONO)
                        .color(u.faint),
                ),
        )
        .child(
            col()
                .w_full()
                .gap(6.0)
                .child(u.field_label("PAYLOAD"))
                .child(
                    u.well_field().w_full().padding(11.0).child(
                        text(pause.payload_text())
                            .font(MONO)
                            .color(u.prose)
                            .wrap_anywhere()
                            .selectable(true),
                    ),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(10.0)
                .center_y()
                .child(
                    u.pill("Approve", "✓", ActionKind::Primary)
                        .key("approve")
                        .autofocus()
                        .on_press(|st: &mut State| st.approve_pause()),
                )
                .child(
                    u.pill("Deny", "esc", ActionKind::Ghost)
                        .key("deny")
                        .on_press(|st: &mut State| st.deny_pause()),
                )
                .child(spacer())
                .child(
                    text(if s.agent.demo {
                        "Demo fixture: no resume is sent."
                    } else {
                        "Approve resumes with a result. Deny resumes with an error."
                    })
                    .font(SMALL)
                    .color(u.faint),
                ),
        );

    u.glass_shell()
        .key("approval-card")
        .w_full()
        .padding(1.0)
        .child(
            u.glass_core()
                .w_full()
                .fill(u.amber.alpha(0.035))
                .border(1.0, u.amber.alpha(0.22))
                .child(inner),
        )
}

fn feed(s: &State, u: &Ui) -> El {
    let mut list = col().key("feed").w_full().gap(22.0).pb(8.0);
    if !s.demo && s.messages.len() <= 1 {
        list = list.child(start_strip(u));
    }
    for (i, m) in s.messages.iter().enumerate() {
        list = list.child(message(s, u, i, m));
    }
    u.glass_shell()
        .key("transcript-shell")
        .grow(1.0)
        .min_h(0.0)
        .padding(1.0)
        .child(
            u.glass_core().grow(1.0).min_h(0.0).child(
                col()
                    .key("transcript")
                    .grow(1.0)
                    .min_h(0.0)
                    .padding(18.0)
                    .clip_soft(14.0)
                    .scroll(ScrollSpec { stick_bottom: true })
                    .child(list),
            ),
        )
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
        .gap(7.0)
        .padding(13.0)
        .radius(13.0)
        .fill(Color::WHITE.alpha(0.022))
        .border(1.0, Color::WHITE.alpha(0.05))
        .hover(Color::WHITE.alpha(0.045))
        .press(Color::WHITE.alpha(0.06))
        .press_scale(0.985)
        .focusable(true)
        .role("button")
        .label(title)
        .motion(motion::SNAPPY)
        .on_press(press)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(u.glyph_box(glyph, Tone::Quiet, 26.0))
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
        .child(spacer())
        .child(
            col()
                .max_w(620.0)
                .gap(6.0)
                .padding(14.0)
                .radius(16.0)
                .fill(Color::WHITE.alpha(0.035))
                .border(1.0, Color::WHITE.alpha(0.055))
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(text("You").font(BODY_MED).color(u.ink))
                        .child(text(m.meta.clone()).font(SMALL).color(u.faint)),
                )
                .child(
                    text(m.body.clone())
                        .font(CHAT)
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
        .child(text("Agent").font(BODY_MED).color(u.ink));
    if let Some(model) = &m.model {
        meta = meta.child(u.chip(model, u.teal, u.teal.alpha(0.10)));
    }
    meta = meta.child(text(m.meta.clone()).font(SMALL).color(u.faint));
    let mut body = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(7.0)
        .max_w(760.0)
        .child(meta)
        .child(
            text(m.body.clone())
                .font(CHAT)
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
        .gap(0.0)
        .items_start()
        .child(body)
}

fn tool_row(_s: &State, u: &Ui, idx: usize, tool: Option<&ToolRun>) -> El {
    let Some(tool) = tool else {
        return el().key(format!("msg-{idx}")).h(0.0);
    };
    let tone = run_tone(tool.state);
    let mut head = row()
        .px(12.0)
        .pt(7.0)
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
        .radius(12.0)
        .fill(Color::WHITE.alpha(0.018))
        .border(1.0, Color::WHITE.alpha(0.04))
        .child(head);
    if !tool.args.is_empty() {
        card = card.child(
            text(tool.args.clone())
                .font(MONO)
                .color(u.faint)
                .px(12.0)
                .wrap_anywhere(),
        );
    }
    card = card.child(
        row()
            .px(12.0)
            .pb(6.0)
            .gap(8.0)
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
        .gap(13.0)
        .items_start()
        .child(el().w(30.0).h(1.0))
        .child(card)
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
    let field = rune::text_input(&s.composer, CHAT)
        .key("composer")
        .label("Prompt")
        .grow(1.0)
        .min_h(26.0)
        .max_h(104.0)
        .color(u.ink)
        .placeholder("Ask the agent, or describe the mod you want", u.faint)
        .caret(u.accent, u.accent.alpha(0.22))
        .on_edit(|st: &mut State| &mut st.composer)
        .on_submit(|st: &mut State, _| st.send_prompt());

    let ready = !s.composer.text.trim().is_empty() && s.agent.status == AgentStatus::Ready;
    let running = s.agent.status == AgentStatus::Running;
    let action = if running {
        u.pill("Stop", "■", ActionKind::Secondary)
            .key("abort")
            .on_press(|st: &mut State| st.abort_run())
    } else {
        let mut send = u
            .pill("Send", "↑", ActionKind::Primary)
            .key("send")
            .on_press(|st: &mut State| st.send_prompt());
        if !ready {
            send = send.opacity(0.52);
        }
        send
    };

    let inner = col()
        .w_full()
        .padding(14.0)
        .gap(9.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .items_end()
                .child(field)
                .child(action),
        )
        .child(
            row()
                .w_full()
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
        );

    u.glass_shell()
        .key("composer-card")
        .w_full()
        .padding(1.0)
        .child(
            u.glass_core()
                .w_full()
                .fill(Color::WHITE.alpha(0.028))
                .child(inner),
        )
}

fn rail(s: &State, u: &Ui) -> El {
    let tone = status_tone(s.agent.status);
    let project = s.project();

    let mut model = col().w_full().gap(9.0).child(
        row()
            .gap(8.0)
            .center_y()
            .child(text("MODEL").font(LABEL).color(u.faint))
            .child(spacer())
            .child(u.dot(tone))
            .child(u.status_chip(s.agent.status.label(), tone)),
    );
    model = model
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
            .wrap_anywhere(),
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
                .gap(8.0)
                .child(text("session").font(SMALL).color(u.faint))
                .child(spacer())
                .child(
                    text(s.agent.session_label())
                        .font(MONO)
                        .color(u.muted)
                        .truncate(),
                ),
        );

    let mut context = col()
        .w_full()
        .gap(8.0)
        .child(text("CONTEXT").font(LABEL).color(u.faint));
    match project {
        Some(p) => {
            context = context
                .child(text(p.name.clone()).font(BODY_MED).color(u.ink))
                .child(text(p.summary.clone()).font(SMALL).color(u.faint).wrap())
                .child(
                    row()
                        .gap(6.0)
                        .child(u.chip(
                            &format!("slot {:02}", p.slot),
                            u.accent,
                            u.accent.alpha(0.11),
                        ))
                        .child(u.chip(
                            &format!("{} targets", p.targets.len()),
                            u.violet,
                            u.violet.alpha(0.10),
                        )),
                );
        }
        None => {
            context = context.child(
                text("No project is open. The session is unscoped, so game paths are guesses.")
                    .font(SMALL)
                    .color(u.muted)
                    .wrap(),
            );
        }
    }
    context = context
        .child(u.divider().mt(4.0))
        .child(boundary_row(u, "Staging folder", "agent", Tone::Ok))
        .child(boundary_row(u, "Game install", "approval", Tone::Warn))
        .child(boundary_row(u, "Publish", "approval", Tone::Warn));

    let mut timeline = col().key("timeline-list").w_full().gap(0.0);
    if s.tools.is_empty() {
        timeline = timeline.padding(2.0).child(
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

    let timeline_block = col()
        .w_full()
        .gap(6.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text("TOOL TIMELINE").font(LABEL).color(u.faint))
                .child(spacer())
                .child(
                    text(format!("{} calls", s.tools.len()))
                        .font(SMALL)
                        .color(u.faint),
                ),
        )
        .child(
            col()
                .w_full()
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(timeline),
        );

    let stats = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            text(format!("{} messages", s.messages.len()))
                .font(SMALL)
                .color(u.muted),
        )
        .child(text("·").font(SMALL).color(u.faint).a11y_hidden())
        .child(
            text(format!("{} tools", s.tools.len()))
                .font(SMALL)
                .color(u.muted),
        )
        .child(text("·").font(SMALL).color(u.faint).a11y_hidden())
        .child(text(s.elapsed.clone()).font(SMALL).color(u.muted))
        .child(spacer())
        .child(text("this session").font(SMALL).color(u.faint));

    let body = col()
        .w_full()
        .padding(16.0)
        .gap(14.0)
        .child(model)
        .child(u.divider())
        .child(context)
        .child(u.divider())
        .child(timeline_block)
        .child(u.divider())
        .child(stats)
        .child(
            text("The running tool drives this timeline. No tool writes to the game install.")
                .font(SMALL)
                .color(u.faint)
                .wrap(),
        );

    col()
        .key("rail")
        .w(RAIL_W)
        .h_full()
        .min_h(0.0)
        .gap(12.0)
        .child(
            u.glass_shell()
                .key("context-rail")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .padding(1.0)
                .child(
                    u.glass_core()
                        .w_full()
                        .grow(1.0)
                        .min_h(0.0)
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(body),
                ),
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
        .h(32.0)
        .px(2.0)
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
