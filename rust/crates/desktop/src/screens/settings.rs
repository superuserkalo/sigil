//! Settings: provider status, the local endpoint, paths and safety. No secret is ever rendered.

use rune::{Color, El, ScrollSpec, col, row, text};

use crate::agent::AgentStatus;
use crate::app::State;
use crate::model::{Provider, ProviderState, Screen};
use crate::ui::{ActionKind, BODY_MED, MONO, SMALL, Tone, Ui};

pub fn view(s: &State, u: &Ui) -> El {
    let configured = s
        .providers
        .iter()
        .filter(|p| p.state == ProviderState::Configured)
        .count();
    let host_tone = host_tone(s.agent.status);
    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(u.status_chip(&format!("host {}", s.agent.status.label()), host_tone))
        .child(u.status_chip(
            &format!("{configured} of {} configured", s.providers.len()),
            if configured > 0 { Tone::Ok } else { Tone::Warn },
        ))
        .child(u.chip("NO SECRETS ON SCREEN", u.faint, Color::WHITE.alpha(0.035)));

    let left = col()
        .key("settings-left")
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .gap(12.0)
        .child(providers(u, s))
        .child(endpoint(s, u))
        .child(paths(s, u));

    let right = col()
        .key("settings-right")
        .w(372.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .gap(12.0)
        .child(safety(s, u))
        .child(agent_host(s, u))
        .child(about(u));

    col()
        .key("settings-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(20.0)
        .pb(18.0)
        .gap(14.0)
        .child(u.page_head(
            Screen::Settings.kicker(),
            "Settings",
            "Providers, the local endpoint, workspace paths and the write boundary.",
            Some(trailing),
        ))
        .child(
            row()
                .key("settings-body")
                .grow(1.0)
                .min_h(0.0)
                .gap(16.0)
                .child(left)
                .child(right),
        )
}

fn providers(u: &Ui, s: &State) -> El {
    let mut body = col().key("provider-list").w_full().padding(6.0);
    for (i, p) in s.providers.iter().enumerate() {
        if i > 0 {
            body = body.child(u.divider());
        }
        body = body.child(provider_row(u, p));
    }
    u.panel()
        .key("providers")
        .w_full()
        .child(u.panel_header("MODEL PROVIDERS", "BYOK · OS keychain"))
        .child(body)
        .child(
            row()
                .w_full()
                .px(14.0)
                .pb(12.0)
                .child(
                    text("Keys are never rendered here. The CLI stores them chmod 600; the app store uses the OS keychain.")
                        .font(SMALL)
                        .color(u.faint)
                        .wrap(),
                ),
        )
}

fn provider_row(u: &Ui, p: &Provider) -> El {
    let tone = match p.state {
        ProviderState::Configured => Tone::Ok,
        ProviderState::NotConfigured => Tone::Quiet,
    };
    let label = match p.state {
        ProviderState::Configured => "configured",
        ProviderState::NotConfigured => "not configured",
    };
    let initial = p
        .name
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "·".to_string());
    row()
        .w_full()
        .key(format!("provider-{}", p.name))
        .padding(10.0)
        .gap(10.0)
        .items_center()
        .child(u.glyph_box(&initial, tone, 30.0))
        .child(
            col()
                .grow(1.0)
                .gap(2.0)
                .child(
                    row()
                        .gap(7.0)
                        .center_y()
                        .child(text(p.name.clone()).font(BODY_MED).color(u.ink))
                        .child(u.chip(&p.kind, u.faint, Color::WHITE.alpha(0.035))),
                )
                .child(text(p.detail.clone()).font(SMALL).color(u.muted).wrap()),
        )
        .child(u.status_chip(label, tone))
}

fn endpoint(s: &State, u: &Ui) -> El {
    let field = rune::text_input(&s.local_endpoint, MONO)
        .key("local-endpoint")
        .label("Local OpenAI compatible base URL")
        .single_line()
        .grow(1.0)
        .color(u.ink)
        .placeholder("http://127.0.0.1:8080/v1", u.faint)
        .caret(u.accent, u.accent.alpha(0.22))
        .on_edit(|st: &mut State| &mut st.local_endpoint);

    u.panel()
        .key("endpoint")
        .w_full()
        .child(u.panel_header("LOCAL OPENAI-COMPATIBLE ENDPOINT", "no key required"))
        .child(
            col()
                .w_full()
                .padding(14.0)
                .gap(9.0)
                .child(u.field_label("BASE URL"))
                .child(
                    row()
                        .w_full()
                        .h(36.0)
                        .px(10.0)
                        .gap(8.0)
                        .center_y()
                        .radius(7.0)
                        .fill(u.well)
                        .border(1.0, u.hair_strong)
                        .child(text("⌁").font(MONO).color(u.faint).a11y_hidden())
                        .child(field),
                )
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .items_center()
                        .child(
                            text("The workbench does not start or install a server. Prompts go to this URL only when you send.")
                                .font(SMALL)
                                .color(u.faint)
                                .grow(1.0)
                                .wrap(),
                        )
                        .child(
                            u.action("Test connection", ActionKind::Secondary)
                                .on_press(|st: &mut State| {
                                    st.status =
                                        "Connection test is not wired in this build. No request was made."
                                            .to_string();
                                }),
                        ),
                ),
        )
}

fn paths(s: &State, u: &Ui) -> El {
    let rows = [
        ("Data", s.paths.data.as_str()),
        ("Workspace", s.paths.workspace.as_str()),
        ("Staging", s.paths.staging.as_str()),
        ("Game", s.paths.game.as_str()),
    ];
    let mut body = col().w_full().padding(14.0).gap(2.0);
    for (i, (k, v)) in rows.into_iter().enumerate() {
        if i > 0 {
            body = body.child(u.divider());
        }
        let color = if v.starts_with("Not") || v.starts_with("No") {
            u.faint
        } else {
            u.ink
        };
        body = body.child(u.kv(k, v, color));
    }
    u.panel()
        .key("paths")
        .w_full()
        .child(u.panel_header("PATHS", "from the settings file"))
        .child(body)
        .child(
            row()
                .w_full()
                .px(14.0)
                .pb(12.0)
                .child(
                    text("Paths are read from the app settings file. Nothing on this screen writes to disk.")
                        .font(SMALL)
                        .color(u.faint)
                        .wrap(),
                ),
        )
}

fn safety(s: &State, u: &Ui) -> El {
    u.panel()
        .key("safety")
        .w_full()
        .child(u.panel_header("SAFETY", "approval gates"))
        .child(
            col()
                .w_full()
                .padding(8.0)
                .child(toggle_row(
                    u,
                    "Confirm before writing to the game",
                    "Deploy, gameinfo repair, launch and publish pause for approval.",
                    s.confirm_writes,
                    |st: &mut State| {
                        st.confirm_writes = !st.confirm_writes;
                        st.status = format!(
                            "Write confirmation {}. Game writes stay approval-gated either way.",
                            if st.confirm_writes { "on" } else { "off" }
                        );
                    },
                ))
                .child(u.divider())
                .child(toggle_row(
                    u,
                    "Re-check impact after patch notes",
                    "A new official note flags targets that correlate with it.",
                    s.auto_recheck,
                    |st: &mut State| st.auto_recheck = !st.auto_recheck,
                ))
                .child(u.divider())
                .child(toggle_row(
                    u,
                    "Check for updates at launch",
                    "Reads the GitHub release feed. No telemetry is sent.",
                    s.check_updates,
                    |st: &mut State| {
                        st.check_updates = !st.check_updates;
                        st.status = if st.check_updates {
                            "Update check is enabled in settings. The feed is not connected in this build."
                                .to_string()
                        } else {
                            "Update check is off.".to_string()
                        };
                    },
                )),
        )
        .child(
            u.note("The agent proposes. The app writes, and only after an approval gate. Toggles are session state in this build."),
        )
}

fn toggle_row(u: &Ui, title: &str, detail: &str, on: bool, press: fn(&mut State)) -> El {
    row()
        .w_full()
        .py(10.0)
        .px(6.0)
        .gap(16.0)
        .items_center()
        .child(
            col()
                .grow(1.0)
                .gap(2.0)
                .child(text(title).font(BODY_MED).color(u.ink))
                .child(text(detail).font(SMALL).color(u.faint).wrap()),
        )
        .child(u.toggle(on).label(title).on_press(press))
}

fn agent_host(s: &State, u: &Ui) -> El {
    let tone = host_tone(s.agent.status);
    let session_count = s.agent.session_ids.len();
    let mut rows = col()
        .w_full()
        .padding(14.0)
        .gap(2.0)
        .child(u.kv("Status", s.agent.status.label(), tone.fg(u)))
        .child(u.divider())
        .child(u.kv("Transport", "dah-agent · JSONL over stdio", u.ink))
        .child(u.divider())
        .child(u.kv(
            "Binary",
            &s.agent.binary_label(),
            path_color(u, s.agent.binary.is_some()),
        ))
        .child(u.divider())
        .child(u.kv("Provider", &s.agent.provider, u.ink))
        .child(u.divider())
        .child(u.kv("Model", &s.agent.model_label(), u.ink));
    if let Some(base_url) = &s.agent.base_url {
        rows = rows
            .child(u.divider())
            .child(u.kv("Base URL", base_url, u.muted));
    }
    rows = rows
        .child(u.divider())
        .child(u.kv("Session", &s.agent.session_label(), u.muted))
        .child(u.divider())
        .child(u.kv("Sessions", &format!("{session_count} stored"), u.muted))
        .child(u.divider())
        .child(u.kv("Journal", &s.state_sessions_dir(), u.muted))
        .child(u.divider())
        .child(u.kv("Usage", &s.agent.usage_label(), u.muted));
    if let Some(pause) = &s.agent.pause {
        rows = rows
            .child(u.divider())
            .child(u.kv("Paused on", &pause.tool_name, u.amber));
    }
    if let Some(previous) = &s.agent.previous_session_id {
        rows = rows
            .child(u.divider())
            .child(u.kv("Previous", previous, u.faint));
    }
    if let Some(error) = &s.agent.error {
        rows = rows
            .child(u.divider())
            .child(text(error).font(MONO).color(u.rose).py(6.0).wrap_anywhere());
    }

    u.panel()
        .key("agent-host")
        .w_full()
        .child(u.panel_header("AGENT HOST", "stdin/stdout · local process"))
        .child(rows)
        .child(
            row()
                .w_full()
                .px(14.0)
                .pb(12.0)
                .gap(8.0)
                .center_y()
                .child(
                    u.action("Restart host", ActionKind::Secondary)
                        .on_press(|st: &mut State| st.restart_host()),
                )
                .child(
                    u.action("List sessions", ActionKind::Ghost)
                        .on_press(|st: &mut State| {
                            if st.agent.list_sessions() {
                                st.status = "Asked the host for stored session ids.".to_string();
                            } else {
                                st.status = st
                                    .agent
                                    .error
                                    .clone()
                                    .unwrap_or_else(|| {
                                        "The agent host is not running.".to_string()
                                    });
                            }
                        }),
                )
                .child(
                    text(if s.agent.demo {
                        "Demo fixture: no process"
                    } else {
                        "One host owns the journal directory"
                    })
                    .font(SMALL)
                    .color(u.faint),
                ),
        )
        .child(u.note(
            "Rune's one-window run has no close hook. Quit chords ask the host to shut down; otherwise the child exits on stdin EOF when this process ends. Restart reuses the same session directory.",
        ))
}

fn path_color(u: &Ui, exists: bool) -> Color {
    if exists { u.ink } else { u.faint }
}

fn host_tone(status: AgentStatus) -> Tone {
    match status {
        AgentStatus::Ready => Tone::Ok,
        AgentStatus::Running => Tone::Warn,
        AgentStatus::Paused => Tone::Warn,
        AgentStatus::Starting => Tone::Info,
        AgentStatus::Error => Tone::Bad,
        AgentStatus::Stopped => Tone::Quiet,
    }
}

fn about(u: &Ui) -> El {
    u.panel()
        .key("about")
        .w_full()
        .child(u.panel_header("ABOUT", "pre-alpha"))
        .child(
            col()
                .w_full()
                .padding(14.0)
                .gap(7.0)
                .child(text("Deadlock Modding Workbench").font(BODY_MED).color(u.ink))
                .child(
                    row()
                        .gap(6.0)
                        .center_y()
                        .child(u.chip("v0.0.0", u.faint, Color::WHITE.alpha(0.035)))
                        .child(u.chip("RUNE · GPU", u.faint, Color::WHITE.alpha(0.035)))
                        .child(u.chip("RUST 1.99.1", u.faint, Color::WHITE.alpha(0.035)))
                        .child(u.chip("MIT", u.faint, Color::WHITE.alpha(0.035))),
                )
                .child(
                    text("No telemetry, no analytics, no heartbeat. Rune, TinyAgent and Grimoire attribution is in THIRD_PARTY_NOTICES.md.")
                        .font(SMALL)
                        .color(u.faint)
                        .wrap(),
                ),
        )
}
