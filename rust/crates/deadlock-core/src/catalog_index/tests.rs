//! Matcher parity with the Worker's reference (`services/telemetry/test/match.spec.ts`, same
//! fixtures and cases), the owner's scoring adjustments, hostile index bodies, and the refresh
//! against a loopback server (never the real network).

use super::matcher::{CLS_TERM, CLS_VISUAL, normalize, token_match, within_one_edit};
use super::*;

use std::io::Write as _;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

// --- a port of the Worker's `buildIndex`, for fixtures ---------------------------------

const DESCRIPTION_STOP: [&str; 36] = [
    "the", "and", "for", "with", "this", "that", "from", "you", "your", "are", "was", "but", "not", "all", "can",
    "has", "have", "its", "will", "just", "also", "into", "out", "use", "any", "one", "mod", "mods", "deadlock",
    "credit", "credits", "thanks", "https", "http", "www", "com",
];

fn description_words(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in normalize(text).split(' ') {
        let digits = !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit());
        if word.len() < 2 || word.len() > 24 || DESCRIPTION_STOP.contains(&word) || (digits && word.len() > 4) {
            continue;
        }
        if !out.iter().any(|w| w == word) {
            out.push(word.to_string());
        }
        if out.len() >= 60 {
            break;
        }
    }
    out
}

#[derive(Default, Clone)]
struct Fixture {
    id: u64,
    title: &'static str,
    cat: Option<&'static str>,
    sub: Option<&'static str>,
    author: Option<&'static str>,
    description: &'static str,
    terms: Vec<(&'static str, u8)>,
}

fn build(fixtures: &[Fixture], with_terms: bool) -> CatalogIndex {
    let rank = |cls: u8| match cls {
        0 => 0,
        1 => 2,
        2 => 1,
        3 => 3,
        _ => 4,
    };
    let mut strings: Vec<String> = Vec::new();
    let mut mods = Vec::new();
    for f in fixtures {
        let mut fields: Vec<(u32, u8)> = Vec::new();
        let mut add = |text: &str, cls: u8| {
            let key = normalize(text);
            if key.is_empty() {
                return;
            }
            let id = match strings.iter().position(|s| *s == key) {
                Some(id) => id as u32,
                None => {
                    strings.push(key);
                    (strings.len() - 1) as u32
                }
            };
            match fields.iter_mut().find(|(known, _)| *known == id) {
                Some(slot) if rank(cls) > rank(slot.1) => slot.1 = cls,
                Some(_) => {}
                None => fields.push((id, cls)),
            }
        };
        for word in description_words(f.description) {
            add(&word, 0);
        }
        let cat = f.cat.unwrap_or("Skins");
        let sub = f.sub.unwrap_or("");
        add(cat, 1);
        add(sub, 1);
        add(f.author.unwrap_or("someone"), 2);
        if cat.eq_ignore_ascii_case("skins") && !sub.is_empty() {
            add(sub, 3);
        }
        if with_terms {
            for (term, cls) in &f.terms {
                add(term, *cls);
            }
        }
        mods.push(IndexMod {
            id: f.id,
            title: f.title.to_string(),
            updated: 1,
            fields: fields.iter().map(|(id, cls)| id * PACK + u32::from(*cls)).collect(),
            ..IndexMod::default()
        });
    }
    CatalogIndex {
        version: SUPPORTED_VERSION,
        built: 1,
        strings,
        mods,
    }
}

fn fixtures() -> Vec<Fixture> {
    let f = |id, title, sub: &'static str, description| Fixture {
        id,
        title,
        sub: Some(sub),
        description,
        ..Fixture::default()
    };
    vec![
        f(1, "GIGN Venator", "Venator", "Counter-Strike 1.6 GIGN model with a gas mask and goggles."),
        Fixture {
            cat: Some("HUD"),
            ..f(2, "Counter Strike", "Character HUD", "Kill feed like an old shooter.")
        },
        f(3, "Strike Team Haze", "Haze", "Tactical vest and helmet."),
        f(4, "Gojo for Mirage", "Mirage", "Satoru Gojo from Jujutsu Kaisen, blindfold included."),
        Fixture {
            author: Some("matrixfan"),
            ..f(5, "Neo trenchcoat Lady Geist", "Lady Geist", "Long black coat and sunglasses.")
        },
        f(6, "Big Floppa McGinnis", "McGinnis", "The caracal meme cat."),
        f(7, "Plague Doctor Abrams", "Abrams", "Beak gas mask, horror outfit."),
        Fixture {
            cat: Some("HUD"),
            ..f(8, "Minimal HUD", "Gameplay", "Clean minimal interface.")
        },
        f(9, "Venator recolor black", "Venator", "All black recolour."),
        // The owner's case: the page never says "Counter-Strike"; only expansion knows CS is that game.
        Fixture {
            terms: vec![("counter strike", CLS_TERM), ("cs", CLS_TERM), ("counter strike 1 6", CLS_TERM)],
            ..f(10, "CS 1.6 Vindicta skin", "Vindicta", "Sniper from the classic.")
        },
        f(11, "Vindicta but pink", "Vindicta", "Pink."),
    ]
}

fn top_in(index: &CatalogIndex, query: &str, n: usize) -> Vec<u64> {
    Matcher::new(index).search(query, n).hits.iter().map(|hit| hit.id).collect()
}

fn top(query: &str, n: usize) -> Vec<u64> {
    top_in(&build(&fixtures(), false), query, n)
}

fn top_expanded(query: &str, n: usize) -> Vec<u64> {
    top_in(&build(&fixtures(), true), query, n)
}

fn set(ids: Vec<u64>) -> std::collections::BTreeSet<u64> {
    ids.into_iter().collect()
}

// --- parity with match.spec.ts ---------------------------------------------------------

#[test]
fn puts_an_exact_title_match_first_then_title_words_then_description_words() {
    assert_eq!(top("counter strike", 3)[0], 2);
    assert!(top("counter strike", 5).contains(&1));
    assert_eq!(set(top("strike", 2)), set(vec![2, 3]));
}

#[test]
fn finds_description_author_and_hero_words() {
    assert_eq!(top("jujutsu kaisen", 3)[0], 4);
    assert_eq!(top("sunglasses", 3)[0], 5);
    assert_eq!(top("matrixfan", 3)[0], 5);
    assert_eq!(set(top("gas mask", 2)), set(vec![1, 7]));
    assert_eq!(top("vindicta pink", 3)[0], 11);
}

#[test]
fn tolerates_one_typo_and_a_prefix() {
    assert_eq!(top("jujutsu kaisne", 3)[0], 4);
    assert!(top("conter strike", 5).contains(&1));
    assert_eq!(top("floppa mcgin", 3)[0], 6);
    assert!(top("counterstrike", 5).contains(&2));
}

#[test]
fn covers_short_forms_and_spelling_variants_with_no_alias_table() {
    assert_eq!(top("mcg", 3)[0], 6);
    assert_eq!(top("geist", 3)[0], 5);
    assert_eq!(top("recolour venator", 3)[0], 9);
    assert_eq!(top("vindict pink", 3)[0], 11);
}

#[test]
fn requires_every_word_then_offers_the_closest_matches_or_nothing() {
    let index = build(&fixtures(), false);
    let matcher = Matcher::new(&index);
    let found = matcher.search("gas mask abrams", 3);
    assert_eq!((found.hits.iter().map(|h| h.id).collect::<Vec<_>>(), found.closest), (vec![7], false));
    // Most of the query is there: the closest match, flagged as such.
    let found = matcher.search("plague doctor abrams unicorn", 3);
    assert_eq!((found.hits.iter().map(|h| h.id).collect::<Vec<_>>(), found.closest), (vec![7], true));
    // Less than half of it: nothing, so the grid shows its empty state.
    let found = matcher.search("gas mask unicorn", 3);
    assert!(found.hits.is_empty() && !found.closest);
    assert!(matcher.search("breaking bad walter", 3).hits.is_empty());
}

#[test]
fn the_owners_case_counter_strike_finds_the_cs_skin_only_with_expansion() {
    assert!(!top("counter strike", 5).contains(&10));
    assert!(top_expanded("counter strike", 5).contains(&10));
    assert_eq!(top_expanded("counter strike vindicta", 3)[0], 10);
    // "cs vindicta" works on the title alone.
    assert_eq!(top("cs vindicta", 3)[0], 10);
}

#[test]
fn edit_distance_helpers() {
    assert!(within_one_edit("strike", "strik"));
    assert!(within_one_edit("kaisen", "kaisne"));
    assert!(!within_one_edit("venator", "vindicta"));
    assert_eq!(token_match("ven", "venator"), 0.8);
    assert_eq!(token_match("ve", "venator"), 0.0);
}

#[test]
fn maps_no_word_to_another() {
    // Without a hand-made list a German word finds only mods that use it.
    assert!(top("katze", 3).is_empty());
}

#[test]
fn normalises_case_accents_and_punctuation() {
    assert_eq!(normalize("  Mo & Krill: Café-Noir!! "), "mo and krill cafe noir");
    assert_eq!(normalize("Ｍｅｉ  Śnieżka"), "mei sniezka");
}

// --- the owner's adjustments -----------------------------------------------------------

fn one(id: u64, title: &'static str, description: &'static str, terms: Vec<(&'static str, u8)>) -> Fixture {
    Fixture {
        id,
        title,
        description,
        terms,
        ..Fixture::default()
    }
}

#[test]
fn a_word_only_in_a_verified_term_finds_the_mod() {
    let index = build(
        &[
            one(1, "Mei as Kelvin", "Ice skin.", vec![("overwatch", CLS_TERM), ("mei", CLS_TERM)]),
            one(2, "Kelvin winter coat", "Warm.", vec![]),
        ],
        true,
    );
    assert_eq!(top_in(&index, "overwatch", 5), vec![1]);
    assert_eq!(top_in(&index, "overwatch kelvin", 5), vec![1]);
}

#[test]
fn generated_terms_shorter_than_three_characters_are_ignored() {
    let index = build(&[one(1, "Sharp textures", "Crisp.", vec![("hd", CLS_TERM), ("hd", CLS_VISUAL)])], true);
    assert!(top_in(&index, "hd", 5).is_empty());
    // A description word of two letters still counts: the rule is for generated terms.
    let index = build(&[one(2, "Sharp textures", "Now in hd.", vec![])], true);
    assert_eq!(top_in(&index, "hd", 5), vec![2]);
}

#[test]
fn visual_terms_count_for_little_and_references_fully() {
    let index = build(
        &[
            one(1, "Alpha", "Plain.", vec![("rifle", CLS_VISUAL)]),
            one(2, "Beta", "Plain.", vec![("rifle", CLS_TERM)]),
            one(3, "Gamma", "A rifle.", vec![]),
            one(4, "Delta", "Plain.", vec![]),
        ],
        true,
    );
    let hits = Matcher::new(&index).search("rifle", 5).hits;
    let score = |id| hits.iter().find(|hit| hit.id == id).map(|hit| hit.score).unwrap();
    assert_eq!(hits[0].id, 2);
    assert!(score(1) < score(3), "a visual term counts less than a description word");
    assert!(score(3) < score(2));
}

#[test]
fn title_matches_always_outrank_generated_term_matches() {
    let index = build(
        &[
            one(1, "Overwatch style HUD", "Plain.", vec![]),
            one(2, "Mei as Kelvin", "Plain.", vec![("overwatch", CLS_TERM), ("overwatch 2", CLS_TERM)]),
            one(3, "Tracer", "Plain.", vec![("overwatch", CLS_TERM)]),
        ],
        true,
    );
    assert_eq!(top_in(&index, "overwatch", 5)[0], 1);
}

#[test]
fn likes_from_the_index_lift_a_score_and_their_absence_changes_nothing() {
    let mut index = build(
        &[one(1, "Red Haze", "Plain.", vec![]), one(2, "Red Haze", "Plain.", vec![]), one(3, "Red Haze", "Plain.", vec![])],
        true,
    );
    let ids = |index: &CatalogIndex| top_in(index, "red haze", 5);
    assert_eq!(ids(&index), vec![1, 2, 3]);
    index.mods[1].likes = Some(90);
    index.mods[2].likes = Some(10);
    assert_eq!(ids(&index), vec![2, 3, 1]);
    // The lift is gentle: a better match still wins over a liked one.
    let mut index = build(&[one(1, "Red Haze jacket", "Plain.", vec![]), one(2, "Haze", "A red one.", vec![])], true);
    index.mods[1].likes = Some(500);
    assert_eq!(top_in(&index, "red haze jacket", 5), vec![1]);
}

#[test]
fn joins_and_splits_compound_words() {
    let index = build(
        &[
            one(1, "Kris from Deltarune", "Plain.", vec![]),
            one(2, "Delta squad armor", "Plain.", vec![]),
            one(3, "Better hit sound", "Plain.", vec![]),
            one(4, "Rat King flag", "Plain.", vec![]),
            one(5, "Counter Strike AWP", "Plain.", vec![]),
        ],
        true,
    );
    // Two typed words that are one on the page, and the other way round.
    assert_eq!(top_in(&index, "delta rune", 5), vec![1]);
    assert_eq!(top_in(&index, "hitsound", 5), vec![3]);
    assert_eq!(top_in(&index, "ratking", 5), vec![4]);
    assert_eq!(top_in(&index, "counterstrike", 5), vec![5]);
}

#[test]
fn non_latin_text_matches_the_raw_title() {
    let index = build(
        &[
            one(1, "Русская озвучка Haze", "Plain.", vec![]),
            one(2, "初音ミク over Ivy", "Plain.", vec![]),
            one(3, "Haze recolor", "Plain.", vec![]),
        ],
        true,
    );
    assert_eq!(top_in(&index, "озвучка", 5), vec![1]);
    assert_eq!(top_in(&index, "РУССКАЯ ОЗВУЧКА", 5), vec![1]);
    assert_eq!(top_in(&index, "初音ミク", 5), vec![2]);
    assert_eq!(top_in(&index, "初音ミク ivy", 5), vec![2]);
    assert!(top_in(&index, "灭霸", 5).is_empty());
    assert!(matcher::is_foreign("хейз") && matcher::is_foreign("胡桃") && !matcher::is_foreign("cafe") && !matcher::is_foreign("café"));
}

#[test]
fn rating_words_filter_on_the_flag_only_when_the_index_has_it() {
    let mut index = build(
        &[
            one(1, "Lewd Haze", "Plain.", vec![]),
            one(2, "Bunny Haze", "Plain.", vec![]),
            one(3, "Bunny Ivy", "Plain.", vec![]),
        ],
        true,
    );
    // Without the key the words are text.
    assert_eq!(top_in(&index, "lewd", 5), vec![1]);
    assert!(!Matcher::new(&index).search("nsfw haze", 5).rated_only);
    index.mods[1].rated = Some(true);
    index.mods[2].rated = Some(true);
    index.mods[2].likes = Some(40);
    let matcher = Matcher::new(&index);
    assert!(matcher.has_ratings());
    let found = matcher.search("nsfw haze", 5);
    assert!(found.rated_only);
    assert_eq!(found.hits.iter().map(|h| h.id).collect::<Vec<_>>(), vec![2]);
    // Alone, the word lists every rated mod, most liked first.
    assert_eq!(top_in(&index, "nsfw", 5), vec![3, 2]);
    assert_eq!(top_in(&index, "18 bunny", 5), vec![3, 2]);
}

#[test]
fn kind_words_in_the_ui_language_map_to_the_english_word() {
    let index = build(
        &[
            Fixture { cat: Some("Maps"), ..one(1, "Street test", "A map.", vec![]) },
            one(2, "Pink crosshair", "Plain.", vec![]),
            one(3, "Haze recolor", "Plain.", vec![]),
        ],
        true,
    );
    let mut matcher = Matcher::new(&index);
    assert!(matcher.search("fadenkreuz", 5).hits.is_empty());
    matcher.set_kind_words([("Fadenkreuz".to_string(), "Crosshair".to_string()), ("Karte".to_string(), "Map".to_string())]);
    let ids = |q: &str| matcher.search(q, 5).hits.iter().map(|h| h.id).collect::<Vec<_>>();
    assert_eq!(ids("fadenkreuz"), vec![2]);
    assert_eq!(ids("karte"), vec![1]);
}

#[test]
fn reads_the_optional_likes_rating_and_added_keys() {
    let body = br#"{"v":3,"built":5,"strings":["haze"],"mods":[
        {"i":7,"ti":"Haze","u":1,"x":[0],"l":12,"n":1,"a":1790000000},
        {"i":8,"ti":"Other","x":[]}]}"#;
    let index = parse(body).unwrap();
    assert_eq!((index.mods[0].likes, index.mods[0].rated, index.mods[0].added), (Some(12), Some(true), Some(1_790_000_000)));
    assert_eq!((index.mods[1].likes, index.mods[1].rated, index.mods[1].added), (None, None, None));
    assert!(parse(br#"{"v":3,"built":5,"strings":[],"mods":[{"i":7,"ti":"x","x":[],"l":-1}]}"#).is_err());
}

#[test]
fn matches_a_2500_mod_index_in_a_few_milliseconds() {
    let words = ["gojo", "kaisen", "venator", "strike", "counter", "mask", "black", "pink", "neon", "gold"];
    let mut strings: Vec<String> = (0..9_000).map(|i| format!("word{i}")).collect();
    strings.extend(words.iter().map(|w| w.to_string()));
    let base = 9_000_u32;
    let mods = (0..2_500_u32)
        .map(|i| IndexMod {
            id: u64::from(i) + 1,
            title: format!("{} skin number {i}", words[(i % 10) as usize]),
            updated: 1,
            fields: (0..34)
                .map(|k| ((i * 37 + k * 101) % 9_000) * PACK)
                .chain([(base + (i % 10)) * PACK + 4])
                .collect(),
            ..IndexMod::default()
        })
        .collect();
    let index = CatalogIndex {
        version: SUPPORTED_VERSION,
        built: 1,
        strings,
        mods,
    };
    let matcher = Matcher::new(&index);
    let start = std::time::Instant::now();
    let runs = 20;
    for _ in 0..runs {
        assert!(!matcher.search("gold skin numbe", 500).hits.is_empty());
    }
    let each = start.elapsed() / runs;
    eprintln!("[catalog_index] one query over 2,500 mods: {each:?}");
    assert!(each < std::time::Duration::from_millis(250), "{each:?}");
}

// --- hostile bodies --------------------------------------------------------------------

#[test]
fn parses_a_good_body_and_drops_field_ids_outside_the_string_table() {
    let body = br#"{"v":3,"built":5,"strings":["haze","skins"],"mods":[
        {"i":7,"ti":"Haze","u":1,"x":[0,9,800,4294967295000]},
        {"i":7,"ti":"Duplicate","u":1,"x":[]},
        {"i":8,"ti":"Other","x":[8],"extra":true}]}"#;
    let index = parse(body).unwrap();
    assert_eq!(index.mods.len(), 2, "the duplicate id is dropped");
    assert_eq!(index.mods[0].fields, vec![0, 9]);
    assert_eq!(index.mods[1].updated, 0);
}

#[test]
fn rejects_wrong_types_versions_and_oversized_parts() {
    let bad: [&[u8]; 9] = [
        b"not json",
        br#"{"v":3,"built":1,"strings":"no","mods":[]}"#,
        br#"{"v":3,"built":1,"strings":[],"mods":[{"i":"7","ti":"x","x":[]}]}"#,
        br#"{"v":3,"built":1,"strings":[1],"mods":[]}"#,
        br#"{"v":3,"built":1,"strings":[],"mods":[{"i":7,"ti":"x","x":[-1]}]}"#,
        br#"{"v":2,"built":1,"strings":[],"mods":[]}"#,
        br#"{"v":3,"built":1,"strings":[],"mods":[{"i":0,"ti":"x","x":[]}]}"#,
        br#"{"built":1,"strings":[],"mods":[]}"#,
        br#"[]"#,
    ];
    for body in bad {
        assert!(parse(body).is_err(), "{}", String::from_utf8_lossy(body));
    }
    let long = format!(r#"{{"v":3,"built":1,"strings":["{}"],"mods":[]}}"#, "a".repeat(MAX_STRING_BYTES + 1));
    assert!(parse(long.as_bytes()).is_err());
    let fields: Vec<String> = (0..=MAX_FIELDS_PER_MOD).map(|_| "0".to_string()).collect();
    let many = format!(r#"{{"v":3,"built":1,"strings":["a"],"mods":[{{"i":1,"ti":"x","x":[{}]}}]}}"#, fields.join(","));
    assert!(parse(many.as_bytes()).is_err());
    let huge = vec![b' '; MAX_BODY_BYTES as usize + 1];
    assert!(parse(&huge).is_err());
}

#[test]
fn a_corrupt_saved_file_reads_as_no_index() {
    let dir = temp("corrupt");
    fs::create_dir_all(super::dir(&dir)).unwrap();
    fs::write(index_path(&dir), b"{\"v\":3,\"built\":1,\"strings\":[").unwrap();
    assert!(load(&dir).is_none());
    assert!(refresh_due(&dir, 1, false));
    let _ = fs::remove_dir_all(&dir);
}

// --- refresh against a loopback server ------------------------------------------------

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dah-catalog-index-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Answers each request with the next canned response and keeps the request heads.
struct Loopback {
    base: String,
    heads: Arc<Mutex<Vec<String>>>,
}

fn serve(responses: Vec<Vec<u8>>) -> Loopback {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let heads = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&heads);
    thread::spawn(move || {
        for response in responses {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let mut buffer = [0_u8; 8192];
            let mut head = Vec::new();
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => head.extend_from_slice(&buffer[..n]),
                }
            }
            seen.lock().unwrap().push(String::from_utf8_lossy(&head).to_string());
            let _ = stream.write_all(&response);
            let _ = stream.flush();
        }
    });
    Loopback { base, heads }
}

fn gzip_response(body: &[u8], etag: &str) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(body).unwrap();
    let zipped = encoder.finish().unwrap();
    let mut out = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Encoding: gzip\r\nETag: {etag}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        zipped.len()
    )
    .into_bytes();
    out.extend_from_slice(&zipped);
    out
}

const GOOD: &[u8] = br#"{"v":3,"built":9,"strings":["overwatch"],"mods":[{"i":5,"ti":"Mei as Kelvin","u":1,"x":[4]}]}"#;

#[test]
fn refresh_saves_a_gzipped_index_then_sends_its_etag_and_keeps_it_on_304() {
    let dir = temp("refresh");
    let server = serve(vec![
        gzip_response(GOOD, "W/\"abc\""),
        b"HTTP/1.1 304 Not Modified\r\nETag: W/\"abc\"\r\nConnection: close\r\n\r\n".to_vec(),
    ]);
    match refresh_from(&dir, &server.base, "1.2.3", 1_000).unwrap() {
        Refresh::Updated(index) => assert_eq!(index.mods[0].title, "Mei as Kelvin"),
        other => panic!("{other:?}"),
    }
    assert_eq!(load(&dir).unwrap().mods.len(), 1);
    assert_eq!(read_meta(&dir).checked_ms, 1_000);
    assert!(!refresh_due(&dir, 1_000 + REFRESH_EVERY_MS - 1, true));
    assert!(refresh_due(&dir, 1_000 + REFRESH_EVERY_MS, true));
    assert!(matches!(refresh_from(&dir, &server.base, "1.2.3", 2_000).unwrap(), Refresh::Unchanged));
    let heads = server.heads.lock().unwrap().clone();
    let first = heads[0].to_ascii_lowercase();
    assert!(first.starts_with("get /v1/catalog/tags "), "{first}");
    assert!(first.contains("x-sigil-client: 1"), "{first}");
    assert!(first.contains("x-sigil-version: 1.2.3"), "{first}");
    assert!(first.contains("user-agent: sigil"), "{first}");
    assert!(!first.contains("if-none-match"), "{first}");
    assert!(heads[1].to_ascii_lowercase().contains("if-none-match: w/\"abc\""), "{}", heads[1]);
    assert_eq!(load(&dir).unwrap().mods.len(), 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_bad_download_keeps_the_previous_file() {
    let dir = temp("keep");
    fs::create_dir_all(super::dir(&dir)).unwrap();
    fs::write(index_path(&dir), GOOD).unwrap();
    let hostile = br#"{"v":3,"built":9,"strings":[],"mods":[{"i":"x"}]}"#;
    let mut oversized = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        MAX_COMPRESSED_BYTES + 1
    )
    .into_bytes();
    oversized.extend_from_slice(b"{}");
    let server = serve(vec![gzip_response(hostile, "\"bad\""), oversized]);
    assert!(refresh_from(&dir, &server.base, "1.0.0", 5).is_err());
    assert!(refresh_from(&dir, &server.base, "1.0.0", 6).is_err());
    assert_eq!(fs::read(index_path(&dir)).unwrap(), GOOD);
    assert_eq!(read_meta(&dir).etag, None);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn refresh_does_nothing_while_offline() {
    let dir = temp("offline");
    crate::net::TEST_OFFLINE.with(|flag| flag.set(true));
    let result = refresh_from(&dir, "http://127.0.0.1:9", "1.0.0", 5);
    crate::net::TEST_OFFLINE.with(|flag| flag.set(false));
    assert!(result.is_err());
    assert!(load(&dir).is_none());
    assert_eq!(read_meta(&dir).checked_ms, 0, "a refused request is not an answer");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn card_rows_round_trip_newest_first_and_ignore_a_corrupt_file() {
    let dir = temp("cards");
    let row = |id: u64| BrowseMod {
        id,
        name: format!("m{id}"),
        profile_url: String::new(),
        model: "Mod".into(),
        author: String::new(),
        category: String::new(),
        subcategory: String::new(),
        sub_category_id: 0,
        version: None,
        updated_at: String::new(),
        likes: id,
        views: 0,
        has_files: true,
        thumbnail_url: None,
        added_ts: 0,
        updated_ts: 0,
        nsfw: false,
        audio_url: None,
        featured: false,
        comments: 0,
        author_id: 0,
        obsolete: false,
    };
    let cards: HashMap<u64, CachedCard> = (1..=3).map(|id| (id, CachedCard { fetched: id as i64, row: row(id) })).collect();
    write_cards(&dir, &cards).unwrap();
    assert_eq!(read_cards(&dir), cards);
    fs::write(cards_path(&dir), b"[{").unwrap();
    assert!(read_cards(&dir).is_empty());
    let _ = fs::remove_dir_all(&dir);
}

/// Times and prints queries over a real downloaded index: `DAH_CATALOG_INDEX=<index.json>
/// cargo test -p deadlock-core real_index -- --ignored --nocapture`.
#[test]
#[ignore]
fn real_index_queries() {
    let Ok(path) = std::env::var("DAH_CATALOG_INDEX") else { return };
    let index = parse(&fs::read(path).unwrap()).unwrap();
    let start = std::time::Instant::now();
    let matcher = Matcher::new(&index);
    eprintln!("prepared {} mods in {:?}", matcher.len(), start.elapsed());
    for query in ["overwatch", "counter strike", "gojo", "gas mask", "zenless zone zero", "mei", "peaky blinders", "hd"] {
        let start = std::time::Instant::now();
        let hits = matcher.search(query, 500).hits;
        let took = start.elapsed();
        let titles: Vec<String> = hits
            .iter()
            .take(5)
            .filter_map(|hit| index.mods.iter().find(|m| m.id == hit.id).map(|m| format!("{} ({})", m.title, hit.score)))
            .collect();
        eprintln!("{query:?}: {} hits in {took:?}: {titles:?}", hits.len());
    }
}

// --- cards read by id ------------------------------------------------------------------

#[test]
fn parses_mod_multi_rows_as_cards_and_skips_hidden_ones() {
    use crate::gamebanana::{missing_record, multi_url, parse_multi};
    let body = r#"[
      {"_idRow":556230,"_sName":"Shark Mo&Krill","_sProfileUrl":"https:\/\/gamebanana.com\/mods\/556230",
       "_sModelName":"Mod","_tsDateAdded":1731795084,"_tsDateModified":1795438342,"_tsDateUpdated":1792269969,
       "_bIsNsfw":false,"_sVersion":"1.3","_nLikeCount":49,"_nViewCount":43820,"_nPostCount":12,"_bIsObsolete":false,
       "_aSubmitter":{"_idRow":3972551,"_sName":"Pinewabble"},
       "_aRootCategory":{"_idRow":33295,"_sName":"Skins"},
       "_aCategory":{"_idRow":33319,"_sName":"Mo & Krill","_sProfileUrl":"https:\/\/gamebanana.com\/mods\/cats\/33319"},
       "_aPreviewMedia":{"_aImages":[{"_sType":"screenshot","_sBaseUrl":"https:\/\/images.gamebanana.com\/img\/ss\/mods","_sFile":"a.jpg","_sFile530":"a.jpg"}]},
       "_bIsTrashed":false,"_bIsWithheld":false,"_bIsPrivate":false},
      {"_idRow":745272,"_sName":"Mature","_sModelName":"Mod","_bIsNsfw":true,"_nLikeCount":3},
      {"_idRow":9,"_sName":"Gone","_sModelName":"Mod","_bIsTrashed":true},
      {"_idRow":10,"_sName":"Hidden","_sModelName":"Mod","_bIsWithheld":true},
      "not an object"
    ]"#;
    let rows = parse_multi(body).unwrap();
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![556230, 745272]);
    let shark = &rows[0];
    assert_eq!((shark.author.as_str(), shark.author_id, shark.likes), ("Pinewabble", 3972551, 49));
    assert_eq!((shark.category.as_str(), shark.subcategory.as_str(), shark.sub_category_id), ("Skins", "Mo & Krill", 33319));
    assert!(shark.thumbnail_url.as_deref().is_some_and(|url| url.contains("a.jpg")));
    assert!(!shark.nsfw && rows[1].nsfw);
    let refused = r#"{"_sErrorCode":"INPUT_ERRORS","_aErrorData":{"_csvRowIds":{"_sErrorCode":"NO_SUCH_RECORD","_sErrorMessage":"Record Mod.99999999 doesn't exist"}}}"#;
    assert_eq!(missing_record(refused), Some(99999999));
    assert!(parse_multi(refused).is_err());
    let url = multi_url(&(1..=60).collect::<Vec<_>>());
    assert!(url.starts_with("https://gamebanana.com/apiv11/Mod/Multi?_csvRowIds=1,2,"));
    assert!(url.contains(",50&") && !url.contains(",51"), "{url}");
}

/// Reads real cards from GameBanana, one gone id included: `cargo test -p deadlock-core
/// live_mod_multi -- --ignored`.
#[test]
#[ignore]
fn live_mod_multi() {
    let rows = crate::gamebanana::fetch_mods_by_id(&[556230, 99_999_999, 745272]).unwrap();
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>().len(), 2, "{rows:?}");
    assert!(rows.iter().all(|r| !r.name.is_empty() && r.thumbnail_url.is_some()));
}
