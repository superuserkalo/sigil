//! Where each config comes from. Only identity lives here: the repository, the
//! folder, who made it and how heavy it is. The settings themselves are fetched
//! from the author's repository at runtime (see `fetch`).

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// How a source marks a release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Versioning {
    /// GitHub releases: the newest published release is the version.
    Tag,
    /// No tags: the newest commit on the default branch, with the version the
    /// file states in its header.
    Commit,
}

#[derive(Debug, Clone, Copy)]
pub struct SourceDef {
    pub id: &'static str,
    pub repo: &'static str,
    pub name: &'static str,
    pub license: &'static str,
    pub versioning: Versioning,
    /// The author asks for their video values as shipped (no caps).
    pub verbatim_video: bool,
}

impl SourceDef {
    pub fn url(&self) -> String {
        format!("https://github.com/{}", self.repo)
    }
}

/// Lighter cuts first. The tier is the one thing a player needs to pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tier {
    Light,
    Balanced,
    MaxFps,
    Potato,
}

impl Tier {
    pub const ALL: [Tier; 4] = [Tier::Light, Tier::Balanced, Tier::MaxFps, Tier::Potato];

    pub fn label(self) -> &'static str {
        match self {
            Tier::Light => "Light",
            Tier::Balanced => "Balanced",
            Tier::MaxFps => "Max FPS",
            Tier::Potato => "Potato",
        }
    }

    /// What the tier costs, in one plain sentence.
    pub fn blurb(self) -> &'static str {
        match self {
            Tier::Light => "A small gain. The game looks almost the same.",
            Tier::Balanced => "A solid gain. Some effects and shadows get simpler.",
            Tier::MaxFps => "A large gain. The game looks noticeably flatter.",
            Tier::Potato => "For low-end PCs. Expect the game to look rough.",
        }
    }
}

/// A config hosted on GameBanana instead of a GitHub repository. Sigil never
/// bundles it: the player downloads the author's archive and reviews the file.
#[derive(Debug, Clone, Copy)]
pub struct HostedRef {
    pub mod_id: u64,
    /// The file to take when the mod lists several configs.
    pub file_id: Option<u64>,
    /// Path fragment of the gameinfo variant to pick first.
    pub hint: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct ConfigDef {
    pub id: &'static str,
    pub name: &'static str,
    pub author: &'static str,
    pub tier: Tier,
    pub source: &'static str,
    /// Folders the config has lived in, newest name first.
    pub folders: &'static [&'static str],
    pub file: &'static str,
    /// video.txt files next to it, the default first.
    pub videos: &'static [&'static str],
    /// The author marks it as a work in progress.
    pub testing: bool,
    /// One line on what makes it different, in Sigil's words.
    pub note: &'static str,
    /// Set for a config downloaded from GameBanana, not fetched from GitHub.
    pub gamebanana: Option<HostedRef>,
}

pub struct BaselineDef {
    pub source: &'static str,
    pub path: &'static str,
}

pub const BASELINE_ID: &str = "stock";

/// The stock `gameinfo.gi` OptimizationLock keeps current with game updates.
pub const BASELINE: BaselineDef = BaselineDef {
    source: "optimizationlock",
    path: "clean gameinfo.gi/gameinfo.gi",
};

/// The catalog id of a gameinfo.gi that came inside a mod the player installed
/// (`crate::mod_settings`). Never fetched: it is cached from the mod's own files.
pub const FROM_MOD_ID: &str = "from-mod";

pub const SOURCES: &[SourceDef] = &[
    SourceDef {
        id: "optimizationlock",
        repo: "Sqooky/OptimizationLock",
        name: "OptimizationLock",
        license: "GPL-3.0",
        versioning: Versioning::Commit,
        verbatim_video: false,
    },
    SourceDef {
        id: "gamebanana",
        repo: "",
        name: "GameBanana",
        license: "See the mod page",
        versioning: Versioning::Commit,
        verbatim_video: true,
    },
    SourceDef {
        id: "optilock",
        repo: "dacooderr/OptiLock",
        name: "OptiLock",
        license: "GPL-3.0",
        versioning: Versioning::Tag,
        verbatim_video: true,
    },
    SourceDef {
        id: "mod",
        repo: "",
        name: "A mod you installed",
        license: "See the mod page",
        versioning: Versioning::Commit,
        verbatim_video: true,
    },
];

pub const DEFAULT_ID: &str = "sqooky";

pub const CONFIGS: &[ConfigDef] = &[
    ConfigDef {
        id: "eskay",
        name: "Eskay's config",
        author: "Eskay",
        tier: Tier::Light,
        source: "optimizationlock",
        folders: &["Eskay's config"],
        file: "gameinfo.gi",
        videos: &[],
        testing: false,
        note: "Sqooky's config without the draw distance cuts, so nothing pops in.",
        gamebanana: None,
    },
    ConfigDef {
        id: "sqooky",
        name: "Sqooky's config",
        author: "Sqooky",
        tier: Tier::Balanced,
        source: "optimizationlock",
        folders: &["Sqooky's .gi"],
        file: "gameinfo.gi",
        videos: &[],
        testing: false,
        note: "The main OptimizationLock config. A safe first pick.",
        gamebanana: None,
    },
    ConfigDef {
        id: "optilock-fps",
        name: "OptiLock FPS",
        author: "dacooder",
        tier: Tier::Balanced,
        source: "optilock",
        folders: &[
            "OptiLock FPS Config (Recommended)",
            "OptiLock Balanced FPS Preset",
            "OptiLock FPS Preset",
        ],
        file: "gameinfo.gi",
        videos: &["video.txt"],
        testing: false,
        note: "OptiLock's recommended config. Keeps the game readable.",
        gamebanana: None,
    },
    ConfigDef {
        id: "piggy",
        name: "Piggy's config",
        author: "Piggy",
        tier: Tier::Balanced,
        source: "optimizationlock",
        folders: &["Piggy's config"],
        file: "gameinfo.gi",
        videos: &["video.txt"],
        testing: false,
        note: "The config a Deadlock streamer plays on.",
        gamebanana: None,
    },
    ConfigDef {
        id: "boot",
        name: "boot's Max FPS",
        author: "boot",
        tier: Tier::MaxFps,
        source: "optimizationlock",
        folders: &["boot's maxium fps config"],
        file: "gameinfo.gi",
        videos: &[],
        testing: false,
        note: "Cuts shadows and effects hard for the highest frame rate.",
        gamebanana: None,
    },
    ConfigDef {
        id: "sqooky-max",
        name: "Sqooky's Max FPS",
        author: "Sqooky",
        tier: Tier::MaxFps,
        source: "optimizationlock",
        folders: &[
            "maximumfps or minimum spec config and resources here",
            "test_cfg",
        ],
        file: "gameinfo.gi",
        videos: &["video.txt"],
        testing: true,
        note: "Sqooky's newest tuning. Changes often and gets less testing.",
        gamebanana: None,
    },
    ConfigDef {
        id: "kaizu",
        name: "kaizuchanerus Minimum Spec",
        author: "kaizuchanerus",
        tier: Tier::Potato,
        source: "optimizationlock",
        folders: &["kaizuchanerus minimum spec"],
        file: "gameinfo.gi",
        videos: &[],
        testing: false,
        note: "Built for minimum-spec PCs.",
        gamebanana: None,
    },
    ConfigDef {
        id: "kaizu-extreme",
        name: "kaizuchanerus Extreme Low",
        author: "kaizuchanerus",
        tier: Tier::Potato,
        source: "optimizationlock",
        folders: &["kaizuchanerus minimum spec"],
        file: "gameinfo-EXTREME-LOW.gi",
        videos: &[],
        testing: false,
        note: "Minimum Spec with post-processing and color correction off. Darker and greyer.",
        gamebanana: None,
    },
    ConfigDef {
        id: "optilock-community",
        name: "OptiLock Community",
        author: "OptiLock community",
        tier: Tier::Potato,
        source: "optilock",
        folders: &["OptiLock Community Config"],
        file: "gameinfo.gi",
        videos: &["video.txt"],
        testing: false,
        note: "An ultra-low preset made by OptiLock players. Rough in the hideout, fine in matches.",
        gamebanana: None,
    },
    ConfigDef {
        id: "optilock-potato",
        name: "OptiLock Potato",
        author: "dacooder",
        tier: Tier::Potato,
        source: "optilock",
        folders: &["OptiLock Potato Config", "OptiLock Max FPS Preset"],
        file: "gameinfo.gi",
        videos: &["video.txt"],
        testing: false,
        note: "Adds draw distance culling on top of OptiLock FPS.",
        gamebanana: None,
    },
    ConfigDef {
        id: "optilock-ultra-potato",
        name: "OptiLock Ultra Potato",
        author: "dacooder",
        tier: Tier::Potato,
        source: "optilock",
        folders: &[
            "OptiLock Ultra Potato Config",
            "OptiLock Ultra Potato Config (No Shadows)",
        ],
        file: "gameinfo.gi",
        videos: &["video.txt", "video(No Shadows).txt", "video(Shadows).txt"],
        testing: false,
        note: "Potato pushed further: no tracers, short particle distance.",
        gamebanana: None,
    },
    ConfigDef {
        id: "gb-dyson",
        name: "Dyson's config",
        author: "Dyson",
        tier: Tier::Potato,
        source: "gamebanana",
        folders: &[],
        file: "gameinfo.gi",
        videos: &[],
        testing: false,
        note: "A long-running config kept in several languages. The archive holds other files too.",
        gamebanana: Some(HostedRef {
            mod_id: 616141,
            file_id: None,
            hint: "russian/gameinfo.gi",
        }),
    },
    ConfigDef {
        id: "gb-optimizationdl",
        name: "OptimizationDL",
        author: "back3p",
        tier: Tier::MaxFps,
        source: "gamebanana",
        folders: &[],
        file: "gameinfo.gi",
        videos: &[],
        testing: false,
        note: "A competitive config. Its archive also holds mods, which Sigil leaves alone.",
        gamebanana: Some(HostedRef {
            mod_id: 671812,
            file_id: None,
            hint: "qq/gameinfo.gi",
        }),
    },
    ConfigDef {
        id: "gb-optimisationlock",
        name: "OptimisationLock",
        author: "Piggy",
        tier: Tier::Balanced,
        source: "gamebanana",
        folders: &[],
        file: "gameinfo.gi",
        videos: &[],
        testing: false,
        note: "Piggy's config, reposted on GameBanana. A second file has a minimum-spec config.",
        gamebanana: Some(HostedRef {
            mod_id: 650519,
            file_id: Some(1670703),
            hint: "piggy's config/gameinfo.gi",
        }),
    },
    ConfigDef {
        id: "gb-hanturaya-competitive",
        name: "Hanturaya's competitive",
        author: "Hanturaya",
        tier: Tier::MaxFps,
        source: "gamebanana",
        folders: &[],
        file: "gameinfo.gi",
        videos: &[],
        testing: false,
        note: "A competitive config that changes ConVars only.",
        gamebanana: Some(HostedRef {
            mod_id: 609804,
            file_id: None,
            hint: "gameinfo.gi",
        }),
    },
    ConfigDef {
        id: "gb-max-fps-small-changes",
        name: "boot's Max FPS, small changes",
        author: "boot",
        tier: Tier::Potato,
        source: "gamebanana",
        folders: &[],
        file: "gameinfo.gi",
        videos: &[],
        testing: false,
        note: "boot's highest frame-rate config with the harsher visual cuts set back.",
        gamebanana: Some(HostedRef {
            mod_id: 673077,
            file_id: None,
            hint: "gameinfo.gi",
        }),
    },
    ConfigDef {
        id: "gb-deadlock-competitive",
        name: "Deadlock Competitive",
        author: "SHR1KN",
        tier: Tier::MaxFps,
        source: "gamebanana",
        folders: &[],
        file: "gameinfo.gi",
        videos: &[],
        testing: false,
        note: "A competitive config built for the March 2026 patch. Ships video settings too.",
        gamebanana: Some(HostedRef {
            mod_id: 658776,
            file_id: None,
            hint: "gameinfo.gi",
        }),
    },
    ConfigDef {
        id: "gb-sidelock",
        name: "Sidelock",
        author: "sidelock",
        tier: Tier::Potato,
        source: "gamebanana",
        folders: &[],
        file: "gameinfo.gi",
        videos: &[],
        testing: false,
        note: "Made for Vulkan. The author's license keeps it on GameBanana.",
        gamebanana: Some(HostedRef {
            mod_id: 722944,
            file_id: None,
            hint: "gameinfo.gi",
        }),
    },
    ConfigDef {
        id: FROM_MOD_ID,
        name: "Config from a mod",
        author: "The mod's creator",
        tier: Tier::Balanced,
        source: "mod",
        folders: &[],
        file: "gameinfo.gi",
        videos: &[],
        testing: false,
        note: "The gameinfo.gi and video.txt settings a mod shipped, reviewed when it was installed.",
        gamebanana: None,
    },
];

pub fn source(id: &str) -> &'static SourceDef {
    SOURCES
        .iter()
        .find(|source| source.id == id)
        .or_else(|| super::repos::lookup(id).map(|(_, source)| source))
        .unwrap_or(&SOURCES[0])
}

/// A built-in config, or a custom preset from a GitHub repository once it was loaded
/// (`repos`).
pub fn config(id: &str) -> Option<&'static ConfigDef> {
    CONFIGS
        .iter()
        .find(|config| config.id == id)
        .or_else(|| super::repos::lookup(id).map(|(config, _)| config))
}

/// Whether `id` names a custom preset from a GitHub repository.
pub fn is_custom(id: &str) -> bool {
    id.starts_with(super::repos::ID_PREFIX)
}

/// The version an untagged file states in a comment, such as
/// `OptimizationLock Ver. 3.9` or `-- ver. 1.1 --`.
pub fn stated_version(text: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)\bver(?:sion)?\.?\s*([0-9]+(?:\.[0-9]+)*[a-z]?)\b").expect("version regex")
    });
    text.split('\n')
        .filter_map(|line| line.find("//").map(|at| &line[at + 2..]))
        .find_map(|comment| re.captures(comment).map(|caps| caps[1].to_string()))
}

/// Whether `id` is a config Sigil downloads from GameBanana rather than GitHub.
pub fn is_hosted(id: &str) -> bool {
    config(id).is_some_and(|def| def.gamebanana.is_some())
}

/// The GameBanana reference for a config, when it has one.
pub fn hosted(id: &str) -> Option<HostedRef> {
    config(id).and_then(|def| def.gamebanana)
}
