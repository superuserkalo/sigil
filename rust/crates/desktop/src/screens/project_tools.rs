//! The Projects page's build tools: Pack and Test, compile with Rebuild, Scan and Repair, and
//! bulk rename with a reference check.
//!
//! - Deadlock Mod Manager (`feature/modding-workbench`, behaviour only): the same four actions.
//! - Sigil: every action runs on a worker over `deadlock_core::{project_build, project_test}`.
//!   Only the confirmed Pack and Test install writes to the game; the rest stays in the project.

use std::path::PathBuf;

use deadlock_core::project_build::{self as build, BuildReport, Fix, RenamePlan};
use deadlock_core::project_test::{self, TestBuild};
use rune::{El, Key, KeyEvent, TextEdit, col, row, spacer, text};

use crate::app::State;
use crate::jobs::Job;
use crate::screens::settings::i18n::fill;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, SMALL, Ui};

#[path = "project_tools_de.rs"]
mod de;

fn tr(text: &str) -> &str {
    use crate::screens::settings::i18n::{self, Lang};
    if i18n::current() == Lang::German
        && let Some((_, de)) = de::DE.iter().find(|(en, _)| *en == text)
    {
        return de;
    }
    i18n::tr(text)
}

const SHOWN: usize = 12;

pub enum Outcome {
    Packed(Box<TestBuild>),
    Installed(PathBuf),
    Built(BuildReport),
    Scanned(Vec<Fix>),
    Repaired(PathBuf, usize),
    Undone(usize),
    Planned(RenamePlan),
    Renamed(usize),
}

pub struct Tools {
    job: Job<Outcome>,
    pub test: Option<Box<TestBuild>>,
    pub report: Option<BuildReport>,
    pub fixes: Option<Vec<Fix>>,
    pub undo: Option<PathBuf>,
    pub rename_from: TextEdit,
    pub rename_to: TextEdit,
    pub rename: Option<RenamePlan>,
    pub error: Option<String>,
}

impl Default for Tools {
    fn default() -> Self {
        Tools {
            job: Job::default(),
            test: None,
            report: None,
            fixes: None,
            undo: None,
            rename_from: TextEdit::new(""),
            rename_to: TextEdit::new(""),
            rename: None,
            error: None,
        }
    }
}

fn project_dir(st: &State) -> Option<PathBuf> {
    crate::screens::explorer::project_dirs().get(st.selected_project).cloned()
}

fn run<F>(st: &mut State, work: F)
where
    F: FnOnce(PathBuf) -> Result<Outcome, String> + Send + 'static,
{
    let Some(dir) = project_dir(st) else {
        st.project_tools.error = Some(tr("This project has no folder on disk.").to_string());
        return;
    };
    st.project_tools.error = None;
    let wake = st.wake.clone();
    st.project_tools.job.start(wake, move || work(dir));
}

fn pack_and_test(st: &mut State) {
    let Some(game) = st.game_root.clone() else {
        st.project_tools.error = Some(tr("Set the game folder in Settings first.").to_string());
        return;
    };
    run(st, move |dir| {
        project_test::prepare(&dir, &game).map(|b| Outcome::Packed(Box::new(b))).map_err(|e| e.to_string())
    });
}

fn install(st: &mut State) {
    let (Some(game), Some(test)) = (st.game_root.clone(), st.project_tools.test.take()) else { return };
    run(st, move |dir| {
        project_test::install(&dir, &game, &test).map(|r| Outcome::Installed(r.target)).map_err(|e| e.to_string())
    });
}

fn compile(st: &mut State, rebuild: bool) {
    let Some(gameinfo) = st.toolchain.compile_gameinfo() else {
        st.project_tools.error = Some(tr("Set DAH_CSDK_PATH to your CSDK install first. See Toolchain.").to_string());
        return;
    };
    run(st, move |dir| {
        let output_dir = dir.join(".dah").join("compile");
        let mut one = |source: &std::path::Path| {
            deadlock_core::toolchain::compile(&deadlock_core::toolchain::CompileRequest {
                source: source.to_path_buf(),
                gameinfo: gameinfo.clone(),
                output_dir: output_dir.clone(),
            })
            .map(|result| result.outputs)
        };
        build::run_build(&dir, rebuild, &mut one).map(Outcome::Built).map_err(|e| e.to_string())
    });
}

fn plan_rename(st: &mut State) {
    let from = st.project_tools.rename_from.text.clone();
    let to = st.project_tools.rename_to.text.clone();
    let game = st.game_root.clone();
    run(st, move |dir| {
        let index: Vec<String> = game
            .and_then(|root| deadlock_core::explorer::open(&root.join("game/citadel/pak01_dir.vpk")).ok())
            .map(|listing| listing.files.into_iter().map(|file| file.path).collect())
            .unwrap_or_default();
        build::plan_rename(&dir, &from, &to, &index).map(Outcome::Planned).map_err(|e| e.to_string())
    });
}

pub fn poll(st: &mut State) -> bool {
    let Some(result) = st.project_tools.job.try_take() else { return false };
    let tools = &mut st.project_tools;
    match result {
        Err(error) => tools.error = Some(error),
        Ok(Outcome::Packed(test)) => tools.test = Some(test),
        Ok(Outcome::Installed(target)) => {
            st.status = fill(tr("Installed the test build as {}."), &[&target.display().to_string()]);
            crate::data::reveal_path(st, target.display().to_string());
        }
        Ok(Outcome::Built(report)) => tools.report = Some(report),
        Ok(Outcome::Scanned(fixes)) => tools.fixes = Some(fixes),
        Ok(Outcome::Repaired(undo, count)) => {
            tools.fixes = None;
            tools.undo = Some(undo);
            st.status = fill(tr("Repaired {} file(s). Undo puts them back."), &[&count.to_string()]);
        }
        Ok(Outcome::Undone(count)) => {
            tools.undo = None;
            st.status = fill(tr("Put {} file(s) back."), &[&count.to_string()]);
        }
        Ok(Outcome::Planned(plan)) => tools.rename = Some(plan),
        Ok(Outcome::Renamed(count)) => {
            tools.rename = None;
            st.status = fill(tr("Renamed {} file(s)."), &[&count.to_string()]);
        }
    }
    true
}

fn line(u: &Ui, value: String) -> El {
    text(value).font(MONO).color(u.muted).wrap_anywhere()
}

fn field(u: &Ui, edit: &TextEdit, key: &'static str, label: &'static str, get: fn(&mut State) -> &mut TextEdit) -> El {
    u.search_shell(
        rune::text_input(edit, MONO)
            .key(key)
            .label(tr(label))
            .single_line()
            .color(u.ink)
            .placeholder(tr(label), u.faint)
            .caret(u.accent, u.accent.alpha(0.22))
            .on_edit(get),
    )
}

pub fn section(s: &State, u: &Ui) -> El {
    let v = &s.project_tools;
    let busy = v.job.is_running();
    let button = |label: &str, key: &'static str, kind: ActionKind| {
        u.action(tr(label), kind).key(key).disabled(busy).opacity(if busy { 0.45 } else { 1.0 }).cursor(rune::Cursor::Pointer)
    };
    let actions = row()
        .w_full()
        .gap(8.0)
        .wrap()
        .child(button("Pack and Test", "tools-pack-test", ActionKind::Primary).on_press(pack_and_test))
        .child(button("Compile changed", "tools-compile", ActionKind::Secondary).on_press(|st: &mut State| compile(st, false)))
        .child(button("Rebuild all", "tools-rebuild", ActionKind::Ghost).on_press(|st: &mut State| compile(st, true)))
        .child(button("Scan and Repair", "tools-scan", ActionKind::Secondary).on_press(|st: &mut State| {
            run(st, |dir| build::scan(&dir).map(Outcome::Scanned).map_err(|e| e.to_string()))
        }));
    let mut body = col().key("project-tools").w_full().gap(10.0).child(text(tr("Build tools")).font(BODY_MED).color(u.ink)).child(actions);
    if busy {
        body = body.child(text(tr("Working...")).font(SMALL).color(u.faint));
    }
    if let Some(error) = &v.error {
        body = body.child(text(error.clone()).font(SMALL).color(u.rose).wrap());
    }
    if let Some(report) = &v.report {
        body = body.child(
            text(fill(
                tr("Built {}, unchanged {}, failed {}."),
                &[&report.built.len().to_string(), &report.unchanged.to_string(), &report.failed.len().to_string()],
            ))
            .font(SMALL)
            .color(if report.failed.is_empty() { u.muted } else { u.amber }),
        );
        for (path, why) in report.failed.iter().take(SHOWN) {
            let first = why.lines().next().unwrap_or_default();
            body = body.child(line(u, format!("{path}: {first}")));
        }
    }
    if let Some(fixes) = &v.fixes {
        if fixes.is_empty() {
            body = body.child(text(tr("Nothing to repair.")).font(SMALL).color(u.muted));
        } else {
            for fix in fixes.iter().take(SHOWN) {
                body = body.child(line(u, format!("{} -> {}", fix.from, fix.to)));
            }
            let chosen = fixes.clone();
            body = body.child(row().child(
                button("Repair", "tools-repair", ActionKind::Secondary).on_press(move |st: &mut State| {
                    let chosen = chosen.clone();
                    run(st, move |dir| {
                        build::repair(&dir, &chosen).map(|undo| Outcome::Repaired(undo, chosen.len())).map_err(|e| e.to_string())
                    });
                }),
            ));
        }
    }
    if let Some(undo) = v.undo.clone() {
        body = body.child(row().child(button("Undo repair", "tools-undo", ActionKind::Ghost).on_press(move |st: &mut State| {
            let undo = undo.clone();
            run(st, move |dir| build::undo_repair(&dir, &undo).map(Outcome::Undone).map_err(|e| e.to_string()));
        })));
    }
    body = body
        .child(text(tr("Bulk rename")).font(BODY_MED).color(u.ink))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(field(u, &v.rename_from, "tools-rename-from", "Old game path", |st| &mut st.project_tools.rename_from).grow(1.0).min_w(0.0))
                .child(field(u, &v.rename_to, "tools-rename-to", "New game path", |st| &mut st.project_tools.rename_to).grow(1.0).min_w(0.0))
                .child(button("Check references", "tools-rename-plan", ActionKind::Secondary).on_press(plan_rename)),
        );
    if let Some(plan) = &v.rename {
        body = body.child(
            text(fill(
                tr("Moves {} file(s). {} project line(s) and {} game path(s) still use the old name."),
                &[&plan.moves.len().to_string(), &plan.project_references.len().to_string(), &plan.game_paths.len().to_string()],
            ))
            .font(SMALL)
            .color(if plan.game_paths.is_empty() { u.muted } else { u.amber })
            .wrap(),
        );
        for (file, at) in plan.project_references.iter().take(SHOWN) {
            body = body.child(line(u, format!("{file}:{at}")));
        }
        for path in plan.game_paths.iter().take(SHOWN) {
            body = body.child(line(u, path.clone()));
        }
        let chosen = plan.clone();
        body = body.child(row().child(button("Rename", "tools-rename", ActionKind::Secondary).disabled(busy || plan.moves.is_empty()).on_press(move |st: &mut State| {
            let chosen = chosen.clone();
            run(st, move |dir| build::apply_rename(&dir, &chosen).map(Outcome::Renamed).map_err(|e| e.to_string()));
        })));
    }
    body
}

/// The install confirm for Pack and Test, mounted at the root.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let test = s.project_tools.test.as_ref()?;
    let plan = &test.plan;
    let close = |st: &mut State| st.project_tools.test = None;
    let file = plan.target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut notes = col().w_full().gap(4.0).child(
        text(fill(tr("{} files go into {}."), &[&test.manifest.entries.to_string(), &file])).font(BODY).color(u.prose).wrap(),
    );
    if plan.replace_required {
        notes = notes.child(text(tr("This replaces the file in that slot. A backup is kept in the project.")).font(BODY).color(u.amber).wrap());
    }
    if plan.gameinfo_repair {
        notes = notes.child(text(tr("gameinfo.gi is repaired too, with a backup.")).font(BODY).color(u.amber).wrap());
    }
    for conflict in plan.conflicts.iter().take(SHOWN) {
        notes = notes.child(
            text(fill(tr("{} wins over this mod on {} path(s)."), &[&conflict.file, &conflict.paths.len().to_string()]))
                .font(SMALL)
                .color(u.amber)
                .wrap(),
        );
    }
    for warning in plan.warnings.iter().chain(&test.warnings).take(SHOWN) {
        notes = notes.child(text(warning.clone()).font(SMALL).color(u.muted).wrap_anywhere());
    }
    Some(
        u.dialog_shell()
            .key("tools-install")
            .role("dialog")
            .label(tr("Install the test build"))
            .w(480.0)
            .padding(24.0)
            .gap(16.0)
            .modal(true, close)
            .on_key(move |st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    close(st);
                    true
                } else {
                    false
                }
            })
            .child(text(tr("Install the test build")).font(H1).color(u.ink))
            .child(notes)
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .child(spacer())
                    .child(u.action(tr("Cancel"), ActionKind::Ghost).key("tools-cancel").cursor(rune::Cursor::Pointer).on_press(close))
                    .child(u.action(tr("Install"), ActionKind::Primary).key("tools-install-go").cursor(rune::Cursor::Pointer).on_press(install)),
            ),
    )
}

// --- scenes ----------------------------------------------------------------------------------

fn scene_tools(st: &mut State) {
    st.load_demo();
    st.screen = crate::model::Screen::Projects;
    st.project_tools.report = Some(BuildReport {
        built: vec!["materials/haze_body.vmat".into()],
        unchanged: 14,
        failed: vec![("materials/haze_coat.vmat".into(), "resourcecompiler.exe failed.".into())],
    });
    st.project_tools.fixes = Some(vec![Fix {
        rule: "misplaced_panorama_image".into(),
        from: "staging/panorama/layout/icon_png.vtex_c".into(),
        to: "staging/panorama/images/layout/icon_png.vtex_c".into(),
        why: String::new(),
    }]);
}

fn scene_install(st: &mut State) {
    scene_tools(st);
    let mut manifest = deadlock_core::release::ReleaseManifest::new("haze", "Haze recolor", "0.2.0", 7, "pak07_dir.vpk");
    manifest.entries = 15;
    st.project_tools.test = Some(Box::new(TestBuild {
        dir: PathBuf::from("/scene/test-build"),
        manifest,
        plan: deadlock_core::deploy::DeployPlan {
            slot: 7,
            target: PathBuf::from("/scene/addons/pak07_dir.vpk"),
            chunk_targets: Vec::new(),
            replace_required: true,
            conflicts: vec![deadlock_core::deploy::DeployConflict {
                slot: 3,
                file: "pak03_dir.vpk".into(),
                paths: vec!["materials/haze_body.vmat_c".into()],
            }],
            gameinfo_repair: false,
            warnings: Vec::new(),
            entries: Vec::new(),
        },
        warnings: Vec::new(),
    }));
}

type Scene = (&'static str, fn(&mut State));

pub const SCENES: &[Scene] = &[("project-tools", scene_tools), ("project-tools-install", scene_install)];
