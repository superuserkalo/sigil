//! Fixture scenes for the Performance screen and its demo catalog.
//!
//! The scenes never touch the network or a real install: a demo catalog stands
//! in for the cache, and a scene sets the state a screen state needs.

use std::path::PathBuf;

use rune::TextEdit;

use deadlock_core::performance::{Applied, Catalog, Tier, catalog, exchange};

use crate::app::State;
use crate::model::Screen;

use super::*;

// --- scenes -------------------------------------------------------------------

/// Two edits on the applied config, as if the player had tuned it.
fn edited(s: &mut State) {
    s.performance.edits_loaded = true;
    s.performance
        .edits
        .set("sqooky", "r_citadel_outlines", Some("1"), "0");
    s.performance
        .edits
        .set("sqooky", "citadel_camera_hero_fov", Some("95"), "105");
}

/// The headless scenes. A demo catalog stands in for the GitHub cache so the
/// scenes never touch the network or the real cache.
pub fn scene(s: &mut State, name: &str) {
    s.load_demo();
    s.screen = Screen::Performance;
    s.game_detected = true;
    s.game_root = Some(PathBuf::from("/nonexistent/Deadlock"));
    s.performance.loaded = true;
    s.performance.catalog = Some(demo::catalog());
    s.performance.fetch = Fetch::Idle;
    set_applied(s, Applied::None);
    let on = |s: &mut State, id: &str| {
        let config = config(s, id).cloned().expect("demo config");
        let options = config.default_options();
        set_applied(
            s,
            Applied::Config {
                id: id.to_string(),
                version: config.version.clone(),
                commit: config.commit[..12].to_string(),
                switches: Some((options.opt_ins.clone(), options.engine)),
            },
        );
        if let Some(Ok(status)) = &mut s.performance.status {
            status.values = exchange::effective(&config, &options).into_iter().collect();
        }
    };
    match name {
        "performance-on" => on(s, "optilock-fps"),
        "performance-apply" => {
            on(s, "optilock-fps");
            s.performance.dialog = Some(Dialog::Apply(catalog::DEFAULT_ID.to_string()));
        }
        "performance-detail" => s.performance.open = Some(catalog::DEFAULT_ID.to_string()),
        "performance-detail-on" => {
            on(s, "optilock-fps");
            if let Some(Ok(status)) = &mut s.performance.status {
                status.video_path = Some(PathBuf::from(
                    "/Steam/userdata/1/1422450/local/cfg/video.txt",
                ));
            }
            s.performance.open = Some("optilock-fps".to_string());
            let config = config(s, "optilock-fps").cloned().expect("demo config");
            let mut options = config.default_options();
            options.opt_ins.remove("citadel_camera_hero_fov");
            s.performance.options.insert("optilock-fps".into(), options);
        }
        "performance-offline" => {
            s.performance.fetch = Fetch::Failed("GitHub could not be reached.".into());
            s.performance.tier = Some(Tier::Potato);
            if let Some(catalog) = &mut s.performance.catalog {
                catalog.checked_at = Some("2026-11-14T08:00:00.000Z".into());
            }
        }
        "performance-fetching" => {
            s.performance.catalog = Some(Catalog::default());
            s.performance.fetch = Fetch::Running;
        }
        "performance-first-offline" => {
            s.performance.catalog = Some(Catalog::default());
            s.performance.fetch = Fetch::Failed("GitHub could not be reached.".into());
        }
        "performance-editor" => {
            on(s, "sqooky");
            edited(s);
            s.performance.editor = Some(editor::Editor {
                config: "sqooky".into(),
                ..Default::default()
            });
        }
        "performance-editor-history" => {
            on(s, "sqooky");
            edited(s);
            if let Some(editor) = s.performance.editor.as_mut() {
                editor.config = "sqooky".into();
                editor.tab = editor::Tab::History;
            } else {
                s.performance.editor = Some(editor::Editor {
                    config: "sqooky".into(),
                    tab: editor::Tab::History,
                    ..Default::default()
                });
            }
        }
        "performance-import" => {
            on(s, "sqooky");
            s.performance.editor = Some(editor::Editor {
                config: "sqooky".into(),
                dialog: Some(editor::Dialog::Review {
                    path: "sqooky-tweaks.cfg".into(),
                    read: exchange::parse(
                        "r_shadows 0\nfps_max 240\ncitadel_camera_hero_fov 105\nr_added_by_player 1\n",
                    ),
                }),
                ..Default::default()
            });
        }
        "performance-import-paste" => {
            on(s, "sqooky");
            s.performance.editor = Some(editor::Editor {
                config: "sqooky".into(),
                dialog: Some(editor::Dialog::Import {
                    field: TextEdit::new("r_shadows 0\nfps_max 240"),
                    error: None,
                }),
                ..Default::default()
            });
        }
        "performance-edit-value" => {
            on(s, "sqooky");
            edited(s);
            s.performance.editor = Some(editor::Editor {
                config: "sqooky".into(),
                dialog: Some(editor::Dialog::Value {
                    key: "r_citadel_outlines".into(),
                    field: TextEdit::new("0"),
                    error: None,
                }),
                ..Default::default()
            });
        }
        "performance-hosted-files" => {
            s.performance.hosted = Some(hosted::demo("gb-hanturaya-competitive"));
        }
        "performance-hosted-review" => {
            s.performance.hosted = Some(hosted::demo_review("gb-hanturaya-competitive"));
        }
        _ => {}
    }
}


mod demo {
    //! A stand-in catalog for the scenes, shaped like the real configs (counts from
    //! a live fetch on 2026-11-17). Values are placeholders, not the authors'.

    use deadlock_core::performance::fetch::CachedConfig;
    use deadlock_core::performance::{
        Body, Catalog, Config, Group, Held, OptIn, SectionOp, VideoSet, catalog,
    };

    struct Shape {
        id: &'static str,
        version: &'static str,
        date: &'static str,
        engine: usize,
        convars: usize,
        visibility: usize,
        camera: usize,
        devtools: usize,
        ignored: usize,
        video: usize,
    }

    const SHAPES: &[Shape] = &[
        Shape {
            id: "eskay",
            version: "1.1",
            date: "2026-10-05",
            engine: 61,
            convars: 351,
            visibility: 6,
            camera: 4,
            devtools: 1,
            ignored: 22,
            video: 0,
        },
        Shape {
            id: "sqooky",
            version: "3.9",
            date: "2026-11-04",
            engine: 62,
            convars: 353,
            visibility: 6,
            camera: 4,
            devtools: 1,
            ignored: 22,
            video: 0,
        },
        Shape {
            id: "optilock-fps",
            version: "v6.1d",
            date: "2026-11-01",
            engine: 46,
            convars: 416,
            visibility: 5,
            camera: 2,
            devtools: 2,
            ignored: 18,
            video: 31,
        },
        Shape {
            id: "piggy",
            version: "bb69d9d",
            date: "2026-10-05",
            engine: 40,
            convars: 300,
            visibility: 4,
            camera: 3,
            devtools: 0,
            ignored: 15,
            video: 24,
        },
        Shape {
            id: "boot",
            version: "2",
            date: "2026-09-04",
            engine: 30,
            convars: 280,
            visibility: 7,
            camera: 0,
            devtools: 0,
            ignored: 19,
            video: 0,
        },
        Shape {
            id: "sqooky-max",
            version: "1.8",
            date: "2026-11-02",
            engine: 61,
            convars: 356,
            visibility: 6,
            camera: 4,
            devtools: 1,
            ignored: 22,
            video: 28,
        },
        Shape {
            id: "kaizu",
            version: "347a669",
            date: "2026-10-21",
            engine: 7,
            convars: 249,
            visibility: 2,
            camera: 0,
            devtools: 0,
            ignored: 12,
            video: 0,
        },
        Shape {
            id: "kaizu-extreme",
            version: "347a669",
            date: "2026-10-21",
            engine: 7,
            convars: 255,
            visibility: 2,
            camera: 0,
            devtools: 0,
            ignored: 12,
            video: 0,
        },
        Shape {
            id: "optilock-community",
            version: "v6.1d",
            date: "2026-11-01",
            engine: 46,
            convars: 420,
            visibility: 5,
            camera: 2,
            devtools: 2,
            ignored: 18,
            video: 31,
        },
        Shape {
            id: "optilock-potato",
            version: "v6.1d",
            date: "2026-11-01",
            engine: 46,
            convars: 416,
            visibility: 5,
            camera: 2,
            devtools: 2,
            ignored: 18,
            video: 31,
        },
        Shape {
            id: "optilock-ultra-potato",
            version: "v6.1d",
            date: "2026-11-01",
            engine: 46,
            convars: 416,
            visibility: 6,
            camera: 2,
            devtools: 2,
            ignored: 18,
            video: 31,
        },
    ];

    const VISIBILITY: &[(&str, &str, Option<&str>)] = &[
        ("r_citadel_outlines", "0", Some("1")),
        ("citadel_trooper_glow_disabled", "1", None),
        ("citadel_unit_status_use_new", "1", None),
        ("r_drawtracers_firstperson", "false", None),
        ("cl_glow_brightness", "1.5", None),
        ("citadel_unit_status_hide_names", "1", None),
        ("r_citadel_glow_health_bars", "0", Some("1")),
    ];
    const CAMERA: &[(&str, &str, Option<&str>)] = &[
        ("citadel_camera_hero_fov", "95", None),
        ("citadel_reduce_camera_shake", "1", None),
        ("citadel_camera_pitch_max", "89", Some("75")),
        ("citadel_camera_pitch_default", "0", None),
    ];
    const DEVTOOLS: &[(&str, &str, Option<&str>)] = &[
        ("citadel_hideout_enable_testing_tools", "1", None),
        ("movement_stats_debug_draw", "1", None),
    ];
    const IGNORED: &[&str] = &[
        "r_shadows",
        "r_drawparticles",
        "r_postprocess_enable",
        "r_texture_lod_scale",
        "r_particle_multiplier",
        "r_drawskybox",
        "csm_max_shadow_dist_override",
        "lb_enable_sunlight",
        "r_citadel_gpu_culling",
        "sc_clutter_enable",
        "r_citadel_depthoffield_enable",
        "lb_enable_stationary_lights",
        "r_nearz",
        "r_RainParticleDensity",
        "lb_enable_baked_shadows",
        "r_citadel_distancefield_ao",
        "r_citadel_cloak_quality",
        "r_particle_cables_enable",
        "sc_fade_distance_scale_override",
        "lb_debug_draw",
        "mat_fullbright",
        "r_citadel_fog",
    ];
    const VIDEO: &[(&str, &str)] = &[
        ("cpu_level", "0"),
        ("gpu_level", "0"),
        ("gpu_mem_level", "0"),
        ("mem_level", "0"),
        ("mat_viewportscale", "1.000000"),
        ("r_citadel_antialiasing", "0"),
        ("r_citadel_upscaling", "0"),
        ("r_effects_bloom", "false"),
        ("r_post_bloom", "false"),
        ("r_depth_of_field", "false"),
        ("r_citadel_motion_blur", "0"),
        ("r_low_latency", "1"),
        ("r_citadel_ssao_quality", "0"),
        ("r_citadel_shadow_quality", "0"),
        ("r_citadel_fog_quality", "0"),
        ("r_texture_stream_mip_bias", "4"),
        ("r_arealights", "0"),
        ("r_reduce_flash", "1"),
    ];
    const SECTIONS: &[&str] = &[
        "RenderSystem",
        "SceneSystem",
        "WorldRenderer",
        "Particles",
        "Engine2",
    ];

    fn opt_ins(list: &[(&str, &str, Option<&str>)], n: usize, group: Group) -> Vec<OptIn> {
        list.iter()
            .take(n)
            .map(|(key, value, stock)| OptIn {
                key: key.to_string(),
                value: value.to_string(),
                group,
                stock: stock.map(str::to_string),
            })
            .collect()
    }

    pub fn catalog() -> Catalog {
        let mut out = Catalog {
            checked_at: Some("2026-11-17T08:00:00.000Z".into()),
            game_convars: true,
            ..Catalog::default()
        };
        for shape in SHAPES {
            let def = catalog::config(shape.id).expect("known config");
            let source = catalog::source(def.source);
            let ignored: Vec<String> = IGNORED
                .iter()
                .take(shape.ignored)
                .map(|k| k.to_string())
                .collect();
            let mut convars: Vec<(String, String)> = (0..shape.convars)
                .map(|i| (format!("demo_setting_{i}"), "0".into()))
                .collect();
            convars.extend(ignored.iter().map(|k| (k.clone(), "0".into())));
            let mut opt = opt_ins(VISIBILITY, shape.visibility, Group::Visibility);
            opt.extend(opt_ins(CAMERA, shape.camera, Group::Camera));
            opt.extend(opt_ins(DEVTOOLS, shape.devtools, Group::Devtools));
            let body = Body {
                section_ops: (0..shape.engine)
                    .map(|i| SectionOp {
                        path: vec![SECTIONS[i % SECTIONS.len()].to_string()],
                        key: format!("DemoKey{i}"),
                        value: Some("0".into()),
                        remove: false,
                    })
                    .collect(),
                convars,
                opt_ins: opt,
                held: vec![
                    Held {
                        key: "r_lightmap_size".into(),
                        place: "ConVars".into(),
                        reason:
                            "Small values turn the map nearly black since the October 2026 update."
                                .into(),
                    },
                    Held {
                        key: "FileSystem".into(),
                        place: "3 settings".into(),
                        reason: "Load order for mods and game content. Sigil keeps yours.".into(),
                    },
                ],
            };
            let mut videos = Vec::new();
            if shape.video > 0 {
                let settings: Vec<(String, String)> = VIDEO
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                if shape.id == "optilock-ultra-potato" {
                    videos.push(VideoSet {
                        file: "video.txt".into(),
                        label: "Shadows".into(),
                        settings: settings.clone(),
                    });
                    videos.push(VideoSet {
                        file: "video(No Shadows).txt".into(),
                        label: "No shadows".into(),
                        settings,
                    });
                } else {
                    videos.push(VideoSet {
                        file: "video.txt".into(),
                        label: "Default".into(),
                        settings,
                    });
                }
            }
            let meta = CachedConfig {
                id: shape.id.into(),
                version: shape.version.into(),
                commit: "77489664a1b2c3d4e5f60718293a4b5c6d7e8f90".into(),
                date: shape.date.into(),
                path: format!("{}/{}", def.folders[0], def.file),
                blob: String::new(),
                videos: Vec::new(),
            };
            out.configs.push(Config {
                def,
                source,
                version: meta.version,
                commit: meta.commit,
                date: meta.date,
                path: meta.path,
                body,
                videos,
                ignored,
                unknown: (0..3).map(|i| format!("demo_unknown_{i}")).collect(),
            });
        }
        out.configs.sort_by_key(|config| config.def.tier);
        out
    }
}
