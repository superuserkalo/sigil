use std::sync::atomic::AtomicBool;

use super::*;

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dah-bench-session-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A game root with `slots` installed (as plain files: the rename does not read them).
fn game(tag: &str, slots: &[u16]) -> (PathBuf, PathBuf) {
    let root = temp(tag);
    let game = root.join("Deadlock");
    let addons = game.join("game/citadel/addons");
    fs::create_dir_all(&addons).unwrap();
    fs::create_dir_all(game.join("game/citadel/cfg")).unwrap();
    for slot in slots {
        fs::write(addons.join(format!("pak{slot:02}_dir.vpk")), b"not a real vpk").unwrap();
    }
    let data = root.join("data");
    fs::create_dir_all(&data).unwrap();
    (game, data)
}

fn prep(variant: Variant) -> PassPrep {
    PassPrep {
        run_id: "b1".into(),
        pass: 2,
        demo: Some("replays/sigil_benchmark".into()),
        copy_from: None,
        window: Some((1000, 6760)),
        variant,
        autoexec_fallback: false,
        video_backup: None,
    }
}

#[test]
fn the_cfg_holds_release_commands_only() {
    let text = cfg_text(&prep(Variant::AsIs));
    assert!(text.starts_with(CFG_HEADER));
    assert!(text.contains("echo \"SIGIL_BENCHMARK b1 2\""));
    assert!(text.contains("timedemo_start 1000\ntimedemo_end 6760\ntimedemoquit replays/sigil_benchmark\n"));
    for cheat in ["sv_cheats", "setpos", "host_timescale", "fps_max 0"] {
        assert!(!text.contains(cheat), "{cheat}");
    }
    let mut whole = prep(Variant::AsIs);
    whole.window = None;
    assert!(cfg_text(&whole).contains("timedemo_start -1\ntimedemo_end -1\n"));
    let mut play = prep(Variant::AsIs);
    play.demo = None;
    assert!(!cfg_text(&play).contains("timedemo"));
    assert!(cfg_text(&play).contains("cl_showfps 4"));
}

#[test]
fn the_autoexec_block_goes_in_and_out_cleanly() {
    let mine = "bind f5 \"say hi\"\n";
    let on = set_autoexec_block(mine, true).unwrap();
    assert_eq!(on, format!("bind f5 \"say hi\"\n{AUTOEXEC_BEGIN}\nexec sigil_benchmark\n{AUTOEXEC_END}\n"));
    assert_eq!(set_autoexec_block(&on, true), None, "already there");
    assert_eq!(set_autoexec_block(&on, false).unwrap(), mine);
    assert_eq!(set_autoexec_block(mine, false), None);
    assert_eq!(set_autoexec_block("", true).unwrap(), format!("{AUTOEXEC_BEGIN}\nexec sigil_benchmark\n{AUTOEXEC_END}\n"));
}

#[test]
fn a_pass_restores_every_change_it_made() {
    let (game, data) = game("restore", &[1, 2, 3]);
    fs::write(crate::autoexec::autoexec_path(&game), "fps_max 240\n").unwrap();
    crate::servers::write_map_launch(&game, Some("streetball"), true).unwrap();
    let demo = data.join("match.dem");
    fs::write(&demo, vec![7u8; 4096]).unwrap();
    let mut p = prep(Variant::Without(vec![2, 3, 9]));
    p.copy_from = Some(demo.clone());
    p.autoexec_fallback = true;

    let marker = prepare_pass(&game, &data, &p).unwrap();
    assert_eq!(marker.disabled, vec![2, 3], "a slot that is not installed is not touched");
    assert_eq!(marker.map_block.as_deref(), Some("streetball"));
    assert!(cfg_path(&game).is_file());
    assert!(replays_dir(&game).join(DEMO_COPY).is_file());
    let autoexec = fs::read_to_string(crate::autoexec::autoexec_path(&game)).unwrap();
    assert!(autoexec.contains("exec sigil_benchmark") && !autoexec.contains("map streetball"));
    let addons = game.join("game/citadel/addons");
    assert!(addons.join("pak02_dir.vpk.disabled").is_file());
    assert!(prepare_pass(&game, &data, &p).is_err(), "one open pass at a time");
    assert_eq!(blockers(&game, &data, false), vec!["A benchmark pass is still open. Sigil puts it back first."]);

    assert!(restore(&data, true).is_err(), "never while the game runs");
    let done = restore(&data, false).unwrap().unwrap();
    assert!(done.problems.is_empty(), "{:?}", done.problems);
    assert!(read_marker(&data).is_none());
    assert!(!cfg_path(&game).exists());
    assert!(!replays_dir(&game).join(DEMO_COPY).exists());
    assert!(addons.join("pak02_dir.vpk").is_file() && addons.join("pak03_dir.vpk").is_file());
    let autoexec = fs::read_to_string(crate::autoexec::autoexec_path(&game)).unwrap();
    assert!(autoexec.starts_with("fps_max 240\n") && !autoexec.contains("sigil_benchmark"));
    assert_eq!(crate::servers::read_map_launch(&game).as_deref(), Some("streetball"));
    assert_eq!(restore(&data, false).unwrap(), None, "nothing left");
    let _ = fs::remove_dir_all(game.parent().unwrap());
}

#[test]
fn a_vanilla_pass_stashes_and_puts_back() {
    let (game, data) = game("vanilla", &[1, 4]);
    prepare_pass(&game, &data, &prep(Variant::Vanilla)).unwrap();
    let addons = game.join("game/citadel/addons");
    assert!(!addons.join("pak01_dir.vpk").exists());
    assert!(crate::launch::stash_status(&data).is_some());
    let done = restore(&data, false).unwrap().unwrap();
    assert!(done.problems.is_empty(), "{:?}", done.problems);
    assert!(addons.join("pak01_dir.vpk").is_file() && addons.join("pak04_dir.vpk").is_file());
    assert!(crate::launch::stash_status(&data).is_none());
    let _ = fs::remove_dir_all(game.parent().unwrap());
}

#[test]
fn restore_leaves_files_that_are_not_sigils() {
    let (game, data) = game("foreign", &[]);
    prepare_pass(&game, &data, &prep(Variant::AsIs)).unwrap();
    // The player replaced the cfg with their own between the crash and the restore.
    fs::write(cfg_path(&game), "// mine\n").unwrap();
    restore(&data, false).unwrap();
    assert_eq!(fs::read_to_string(cfg_path(&game)).unwrap(), "// mine\n");
    // A cfg that is not Sigil's is never overwritten by a pass.
    assert!(prepare_pass(&game, &data, &prep(Variant::AsIs)).is_err());
    let _ = restore(&data, false);
    let _ = fs::remove_dir_all(game.parent().unwrap());
}

#[test]
fn the_launch_passes_condebug_for_this_launch_only() {
    let args = launch_args(crate::benchmark::Display::Game);
    assert_eq!(&args[..2], &["-applaunch", "1422450"]);
    assert!(args.iter().any(|a| a == "-condebug"));
    assert_eq!(&args[args.len() - 2..], &["+exec", "sigil_benchmark"]);
    let fixed = launch_args(crate::benchmark::Display::Fixed { width: 1920, height: 1080 });
    let at = fixed.iter().position(|a| a == "-w").unwrap();
    assert_eq!(&fixed[at..at + 4], &["-w", "1920", "-h", "1080"]);
    assert!(at < fixed.iter().position(|a| a == "+exec").unwrap());
    assert_eq!(steam_exe(&[PathBuf::from("/nonexistent")]), None);
}

#[test]
fn the_follow_reads_the_result_and_ends_when_the_game_quits() {
    let dir = temp("follow");
    let log = dir.join("console.log");
    fs::write(&log, "old 100 frames 1.000 seconds 100.00 fps (10.00 ms/f) 1.000 fps variability\n").unwrap();
    let limits = Limits { tick: Duration::from_millis(1), check: Duration::from_millis(2), ..Limits::default() };
    let stop = AtomicBool::new(false);
    let mut polls = 0;
    let log2 = log.clone();
    let mut stages = Vec::new();
    let followed = follow(
        &log,
        true,
        limits,
        &stop,
        || {
            polls += 1;
            if polls == 1 {
                let mut f = fs::OpenOptions::new().append(true).open(&log2).unwrap();
                use std::io::Write;
                writeln!(f, "SIGIL_BENCHMARK b1 1\nfps_max = 0\nRequesting timedemo of replays/x\n5760 frames 31.204 seconds 184.59 fps ( 5.42 ms/f) 1.284 fps variability, P5/50/95 frame time 4.1/5.2/7.9 ms").unwrap();
            }
            polls < 3
        },
        |live| stages.push(live.stage),
    );
    assert_eq!(followed.end, End::Exited);
    assert!(followed.cfg_ran());
    assert!(followed.lines.iter().any(|l| l.starts_with("Requesting timedemo")), "the pass log keeps the lines");
    assert_eq!(followed.fps_cap(), Some(0.0));
    assert_eq!(followed.timedemo().unwrap().frames, 5760, "the old result in the log is not read");
    assert!(stages.contains(&Stage::Finishing));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_follow_gives_up_when_the_game_never_starts_or_is_stopped() {
    let dir = temp("never");
    let log = dir.join("console.log");
    let fast = Limits { start: Duration::from_millis(20), tick: Duration::from_millis(1), check: Duration::from_millis(2), ..Limits::default() };
    let stop = AtomicBool::new(false);
    assert_eq!(follow(&log, true, fast, &stop, || false, |_| {}).end, End::NeverStarted);
    let stop = AtomicBool::new(true);
    assert_eq!(follow(&log, true, Limits::default(), &stop, || true, |_| {}).end, End::Stopped);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn showfps_frames_are_aligned_to_the_timedemo() {
    // Loading (long frames), the measured 6 frames of 5 ms, then a quit frame.
    let all = vec![200.0, 150.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 90.0];
    assert_eq!(align(&all, 6, 0.030), vec![5.0; 6]);
    assert_eq!(align(&all, 0, 1.0), all);
    assert_eq!(align(&all, 20, 1.0), all);
}

#[test]
fn frames_come_from_the_best_file_written_during_the_pass() {
    let (game, _data) = game("frames", &[]);
    let since = SystemTime::now() - Duration::from_secs(5);
    let followed = Followed {
        events: vec![LogEvent::Timedemo(TimedemoLine {
            frames: 150,
            seconds: 0.75,
            fps: 200.0,
            ms_per_frame: 5.0,
            variability: 0.1,
            p5_ms: None,
            p50_ms: Some(5.0),
            p95_ms: Some(5.0),
        })],
        end: End::Exited,
        last_line: None,
        lines: Vec::new(),
    };
    assert_eq!(collect_frames(&game, since, &followed).0, FrameSource::SummaryOnly);
    let summary = summary_for(FrameSource::SummaryOnly, &[], &followed).unwrap();
    assert_eq!((summary.avg_fps, summary.low_1), (200.0, None));

    let mut log = String::from("Time,Player 1 Position,Smooth FPS,Frame FPS,Smooth MS,Frame MS,Server Frame MS\n");
    log.push_str("0,0 0 0,10,10,100,100,0\n");
    for i in 0..150 {
        log.push_str(&format!("{i},0 0 0,200,200,5,5,0\n"));
    }
    fs::write(game.join("game/citadel/prof_dl_midtown.csv"), &log).unwrap();
    let (source, frames, file) = collect_frames(&game, since, &followed);
    assert_eq!(source, FrameSource::ShowFps);
    assert!(file.unwrap().ends_with("prof_dl_midtown.csv"));
    assert_eq!(frames, vec![5.0; 150], "the loading frame is cut");

    let mut profile = String::from("demo tick,frame start time,frame start delta,frame swap time,frame swap delta,FPS variability\n\n");
    for i in 0..120 {
        profile.push_str(&format!("{i},0,6.0,0,6.0,0\n"));
    }
    fs::write(game.join("game/citadel/timedemo_profile.csv"), &profile).unwrap();
    let (source, frames, _) = collect_frames(&game, since, &followed);
    assert_eq!(source, FrameSource::TimedemoProfile);
    assert_eq!(frames.len(), 120);
    let _ = fs::remove_dir_all(game.parent().unwrap());
}

#[test]
fn a_fixed_window_pass_puts_the_video_settings_back() {
    let (game, data) = game("video", &[]);
    let video = game.join("game/citadel/cfg/video.txt");
    fs::write(&video, "\"setting.defaultres\" \"2560\"\n").unwrap();
    let mut p = prep(Variant::AsIs);
    p.video_backup = Some((video.clone(), data.join("runs/b1/video-before.txt")));
    prepare_pass(&game, &data, &p).unwrap();
    // The game wrote the fixed window size on exit.
    fs::write(&video, "\"setting.defaultres\" \"1920\"\n").unwrap();
    let done = restore(&data, false).unwrap().unwrap();
    assert!(done.done.iter().any(|d| d.contains("video")));
    assert!(fs::read_to_string(&video).unwrap().contains("2560"));
    let _ = fs::remove_dir_all(game.parent().unwrap());
}

#[test]
fn the_replay_hash_is_checked_before_a_pass() {
    let dir = temp("hash");
    let demo = dir.join("x.dem");
    fs::write(&demo, b"replay one").unwrap();
    let sha = crate::servers::file_sha256(&demo).unwrap();
    assert!(verify_demo(&demo, &sha).is_ok());
    fs::write(&demo, b"replay two").unwrap();
    assert!(verify_demo(&demo, &sha).unwrap_err().to_string().contains("changed since the run started"));
    let _ = fs::remove_dir_all(&dir);
}
