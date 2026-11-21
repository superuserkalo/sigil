//! Fun runs: a random hero, lane, item build and ability order from one seed.
//!
//! Behaviour follows Deadlock Mod Manager's hero build randomizer (GPL-3.0, behaviour only):
//! the same three asset lists from `api.deadlock-api.com`, the same budget and slot rules,
//! a seeded generator so a short code reproduces a roll. The tier shapes, the rule list and
//! the share code are Sigil's own.
//!
//! Data:
//!
//! - Heroes: `/v1/assets/heroes?only_active=true`. A hero is offered when it is player
//!   selectable and not disabled. `in_development` is not a filter: it stays set on heroes
//!   that already shipped.
//! - Items: `/v1/assets/items/by-type/upgrade`. An item can be bought when it is `shopable`
//!   and its cost is above zero and not the 9999 placeholder of unreleased items.
//! - Abilities: `/v1/assets/items/by-type/ability`, for the four signature names.
//!
//! Rules for one roll, in draw order (the order is part of the code's meaning):
//!
//! 1. Hero: uniform over the offered heroes, or the locked hero.
//! 2. Lane: solo or duo.
//! 3. Build: four items in each of weapon, vitality and spirit, filled in a shuffled order.
//!    The pool is the hero's draft list (`item_draft_bucketing`); when the hero has none, the
//!    whole shop. Each category picks a tier shape; a slot tries its tier first and then the
//!    nearest tiers, respects the hero's per tier purchase cap, and keeps enough souls for the
//!    slots after it (the cheapest item times the picks left) inside a 34,000 soul budget.
//!    A pick is weighted by the hero's draft weight, three times for a "Good" item. Picking an
//!    item blocks the item, its components and every item built from it. An imbue item names
//!    the ability it goes on (not the ultimate when the imbue says so).
//! 4. Ability order: twelve points in four abilities at 1, 2 and 5 ability points each, each
//!    step weighted towards the cheaper next upgrade. The ability that maxes first leads.
//! 5. Rules: three of [`RULE_COUNT`] self-imposed rules. Their text lives in the UI.
//!
//! Pure: parsing and rolling never touch the network or the disk. Fetching is
//! [`super::StatsClient::fetch_roll_data`].

use std::collections::{BTreeSet, HashMap};

use serde_json::Value;

use crate::error::{Error, Result};

/// Souls a build may spend.
pub const BUDGET: u32 = 34_000;
/// Items per category.
pub const PER_CATEGORY: usize = 4;
/// Self-imposed rules the UI has text for, numbered from 0.
pub const RULE_COUNT: usize = 12;
/// Rules a roll draws.
pub const RULES_DRAWN: usize = 3;
/// Ability point cost of an ability's first, second and third upgrade.
pub const AP_COSTS: [u32; 3] = [1, 2, 5];
/// The prefix of a share code.
pub const CODE_PREFIX: &str = "DLR-";

/// Four tier steps per category, early item first. Sigil's set: every shape climbs, none
/// skips straight to tier 4 without a tier 2, and three of them end on a tier 4 item.
const SHAPES: [[u32; 4]; 8] = [
    [1, 2, 3, 4],
    [1, 2, 2, 3],
    [1, 1, 2, 3],
    [2, 2, 3, 4],
    [1, 2, 3, 3],
    [2, 3, 3, 4],
    [1, 2, 2, 4],
    [2, 2, 3, 3],
];

// --- data --------------------------------------------------------------------------------------

/// An item category in the shop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Weapon,
    Vitality,
    Spirit,
}

impl Category {
    pub const ALL: [Category; 3] = [Category::Weapon, Category::Vitality, Category::Spirit];

    fn parse(text: &str) -> Option<Category> {
        match text {
            "weapon" => Some(Category::Weapon),
            "vitality" | "armor" => Some(Category::Vitality),
            "spirit" | "tech" => Some(Category::Spirit),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Category::Weapon => "Weapon",
            Category::Vitality => "Vitality",
            Category::Spirit => "Spirit",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Category::Weapon => 0,
            Category::Vitality => 1,
            Category::Spirit => 2,
        }
    }
}

/// One hero the roll can land on.
#[derive(Debug, Clone, PartialEq)]
pub struct RollHero {
    pub id: u32,
    pub class_name: String,
    pub name: String,
    pub role: String,
    pub hero_type: String,
    /// 1 to 3.
    pub complexity: u32,
    pub tags: Vec<String>,
    /// Signature ability class names, slots 1 to 4 (4 is the ultimate).
    pub abilities: [String; 4],
    /// Per category, the purchase cap for tiers 1, 2, 3 and so on.
    pub tier_caps: HashMap<Category, Vec<u32>>,
    /// Item class name to (is "Good", weight).
    pub draft: HashMap<String, (bool, f64)>,
    /// The hero's UI colour.
    pub accent: [u8; 3],
}

/// One item the shop sells.
#[derive(Debug, Clone, PartialEq)]
pub struct RollItem {
    pub id: u64,
    pub class_name: String,
    pub name: String,
    pub category: Category,
    pub tier: u32,
    pub cost: u32,
    pub active: bool,
    pub imbue: Option<String>,
    pub components: Vec<String>,
}

/// Everything a roll draws from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RollData {
    pub heroes: Vec<RollHero>,
    pub items: Vec<RollItem>,
    /// Ability class name to display name.
    pub abilities: HashMap<String, String>,
}

impl RollData {
    pub fn hero(&self, id: u32) -> Option<&RollHero> {
        self.heroes.iter().find(|hero| hero.id == id)
    }

    pub fn ability_name(&self, class_name: &str) -> String {
        self.abilities
            .get(class_name)
            .cloned()
            .unwrap_or_else(|| pretty_class(class_name))
    }
}

fn pretty_class(class_name: &str) -> String {
    let words: Vec<String> = class_name
        .trim_start_matches("ability_")
        .split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect();
    words.join(" ")
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string()
}

/// Parse the hero asset list. Heroes that cannot be played, or that lack four abilities, are
/// left out; a list with none left is an error.
pub fn parse_heroes(json: &str) -> Result<Vec<RollHero>> {
    let rows: Vec<Value> = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("The hero list did not parse: {error}")))?;
    let mut heroes = Vec::new();
    for row in rows {
        let flag = |key: &str| row.get(key).and_then(Value::as_bool).unwrap_or(false);
        if !flag("player_selectable") || flag("disabled") {
            continue;
        }
        let Some(id) = row.get("id").and_then(Value::as_u64) else { continue };
        let name = text(&row, "name");
        if name.is_empty() {
            continue;
        }
        let items = row.get("items");
        let ability = |slot: usize| {
            items
                .and_then(|items| items.get(format!("signature{slot}")))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let abilities = [ability(1), ability(2), ability(3), ability(4)];
        if abilities.iter().any(String::is_empty) {
            continue;
        }
        let mut tier_caps = HashMap::new();
        if let Some(info) = row.get("item_slot_info").and_then(Value::as_object) {
            for (key, value) in info {
                if let Some(category) = Category::parse(key) {
                    let caps = value
                        .get("max_purchases_for_tier")
                        .and_then(Value::as_array)
                        .map(|caps| caps.iter().filter_map(Value::as_u64).map(|cap| cap as u32).collect())
                        .unwrap_or_default();
                    tier_caps.insert(category, caps);
                }
            }
        }
        let mut draft = HashMap::new();
        if let Some(pool) = row.get("item_draft_bucketing").and_then(Value::as_object) {
            for (class_name, entry) in pool {
                let good = entry.get("bucket").and_then(Value::as_str) == Some("Good");
                let weight = entry.get("weight").and_then(Value::as_f64).unwrap_or(1.0);
                draft.insert(class_name.clone(), (good, if weight > 0.0 { weight } else { 1.0 }));
            }
        }
        let accent = row
            .get("colors")
            .and_then(|colors| colors.get("ui"))
            .and_then(Value::as_array)
            .filter(|rgb| rgb.len() == 3)
            .map(|rgb| {
                let c = |i: usize| rgb[i].as_u64().unwrap_or(0).min(255) as u8;
                [c(0), c(1), c(2)]
            })
            .unwrap_or([255, 237, 121]);
        let mut tags: Vec<String> = Vec::new();
        let gun = text(&row, "gun_tag");
        if !gun.is_empty() {
            tags.push(gun);
        }
        for tag in row.get("tags").and_then(Value::as_array).into_iter().flatten() {
            if let Some(tag) = tag.as_str().map(str::trim).filter(|tag| !tag.is_empty())
                && !tags.iter().any(|seen| seen == tag)
            {
                tags.push(tag.to_string());
            }
        }
        heroes.push(RollHero {
            id: id as u32,
            class_name: text(&row, "class_name"),
            name,
            role: row
                .get("description")
                .map(|description| text(description, "role"))
                .unwrap_or_default(),
            hero_type: text(&row, "hero_type"),
            complexity: row.get("complexity").and_then(Value::as_u64).unwrap_or(1).clamp(1, 3) as u32,
            tags,
            abilities,
            tier_caps,
            draft,
            accent,
        });
    }
    heroes.sort_by_key(|hero| hero.id);
    if heroes.is_empty() {
        return Err(Error::msg("The hero list has no playable hero."));
    }
    Ok(heroes)
}

/// Parse the upgrade list, keeping only items the shop sells.
pub fn parse_items(json: &str) -> Result<Vec<RollItem>> {
    let rows: Vec<Value> = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("The item list did not parse: {error}")))?;
    let mut items = Vec::new();
    for row in rows {
        if row.get("shopable").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let cost = row.get("cost").and_then(Value::as_u64).unwrap_or(0) as u32;
        if cost == 0 || cost == 9999 {
            continue;
        }
        let Some(category) = row.get("item_slot_type").and_then(Value::as_str).and_then(Category::parse) else {
            continue;
        };
        let tier = row.get("item_tier").and_then(Value::as_u64).unwrap_or(0) as u32;
        let class_name = text(&row, "class_name");
        let name = text(&row, "name");
        if tier == 0 || class_name.is_empty() || name.is_empty() {
            continue;
        }
        items.push(RollItem {
            id: row.get("id").and_then(Value::as_u64).unwrap_or(0),
            class_name,
            name,
            category,
            tier,
            cost,
            active: row.get("is_active_item").and_then(Value::as_bool).unwrap_or(false),
            imbue: row
                .get("imbue")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|imbue| !imbue.is_empty()),
            components: row
                .get("component_items")
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default(),
        });
    }
    items.sort_by(|a, b| a.class_name.cmp(&b.class_name));
    if items.is_empty() {
        return Err(Error::msg("The item list has no item the shop sells."));
    }
    Ok(items)
}

/// Parse the ability list into class name to display name.
pub fn parse_abilities(json: &str) -> Result<HashMap<String, String>> {
    let rows: Vec<Value> = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("The ability list did not parse: {error}")))?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let class_name = text(row, "class_name");
            let name = text(row, "name");
            (!class_name.is_empty() && !name.is_empty()).then_some((class_name, name))
        })
        .collect())
}

/// The three lists captured in one file (`desktop/fixtures/stats-roll.json`).
pub fn data_from_bundle(json: &str) -> Result<RollData> {
    let value: Value = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("The roll bundle did not parse: {error}")))?;
    let part = |key: &str| value.get(key).map(Value::to_string).unwrap_or_else(|| "[]".to_string());
    Ok(RollData {
        heroes: parse_heroes(&part("heroes"))?,
        items: parse_items(&part("items"))?,
        abilities: parse_abilities(&part("abilities"))?,
    })
}

impl super::StatsClient {
    /// The three asset lists a roll draws from, cached for a week like the other assets.
    /// About 7 MB on a first fetch (the hero list carries lore and art metadata), nothing
    /// about the player is sent.
    pub fn fetch_roll_data(&self) -> Result<RollData> {
        let ttl = super::cache::TTL_ASSETS;
        let heroes = self.get_text("/v1/assets/heroes", &[("only_active", "true".to_string())], ttl)?;
        let items = self.get_text("/v1/assets/items/by-type/upgrade", &[], ttl)?;
        let abilities = self.get_text("/v1/assets/items/by-type/ability", &[], ttl)?;
        Ok(RollData {
            heroes: parse_heroes(&heroes)?,
            items: parse_items(&items)?,
            abilities: parse_abilities(&abilities)?,
        })
    }
}

// --- the generator -----------------------------------------------------------------------------

/// A small seeded generator (mulberry32). The same seed always gives the same stream, which is
/// what makes a share code reproduce a roll.
#[derive(Debug, Clone)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Rng {
        Rng(seed)
    }

    pub fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x6D2B_79F5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        t ^ (t >> 14)
    }

    /// A float in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        f64::from(self.next_u32()) / 4_294_967_296.0
    }

    /// An index below `len` (0 when `len` is 0).
    pub fn below(&mut self, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        ((self.unit() * len as f64) as usize).min(len - 1)
    }

    fn shuffle<T>(&mut self, list: &mut [T]) {
        for index in (1..list.len()).rev() {
            let other = self.below(index + 1);
            list.swap(index, other);
        }
    }

    /// A weighted pick: the index of the chosen weight. `None` for an empty or all zero list.
    fn weighted(&mut self, weights: &[f64]) -> Option<usize> {
        let total: f64 = weights.iter().filter(|weight| **weight > 0.0).sum();
        if total <= 0.0 {
            return None;
        }
        let mut target = self.unit() * total;
        for (index, weight) in weights.iter().enumerate() {
            if *weight <= 0.0 {
                continue;
            }
            if target < *weight {
                return Some(index);
            }
            target -= weight;
        }
        weights.iter().rposition(|weight| *weight > 0.0)
    }
}

/// A fresh seed from the clock, for Roll.
pub fn fresh_seed() -> u32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let mut rng = Rng::new((nanos as u32) ^ ((nanos >> 32) as u32));
    rng.next_u32().max(1)
}

// --- share code ------------------------------------------------------------------------------

/// A seed and, when the player locked one, the hero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Code {
    pub seed: u32,
    pub hero: Option<u32>,
}

fn base36(mut value: u32) -> String {
    const DIGITS: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    if value == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

impl Code {
    /// `DLR-<seed in base 36>`, plus `-H<hero id>` for a locked hero.
    pub fn text(self) -> String {
        match self.hero {
            Some(hero) => format!("{CODE_PREFIX}{}-H{hero}", base36(self.seed)),
            None => format!("{CODE_PREFIX}{}", base36(self.seed)),
        }
    }

    /// Find a code anywhere in pasted text (a chat message, the copied build).
    pub fn find(text: &str) -> Option<Code> {
        let upper = text.to_ascii_uppercase();
        let start = upper.find(CODE_PREFIX)? + CODE_PREFIX.len();
        let rest = &upper[start..];
        let seed_text: String = rest.chars().take_while(char::is_ascii_alphanumeric).collect();
        if seed_text.is_empty() || seed_text.len() > 7 {
            return None;
        }
        let seed = u32::try_from(u64::from_str_radix(&seed_text, 36).ok()?).ok()?;
        let after = &rest[seed_text.len()..];
        let hero = after
            .strip_prefix("-H")
            .map(|tail| tail.chars().take_while(char::is_ascii_digit).collect::<String>())
            .and_then(|digits| digits.parse::<u32>().ok());
        Some(Code { seed, hero })
    }
}

// --- the roll ----------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Solo,
    Duo,
}

impl Lane {
    pub fn label(self) -> &'static str {
        match self {
            Lane::Solo => "Solo lane",
            Lane::Duo => "Duo lane",
        }
    }
}

/// One item in the build, in buying order.
#[derive(Debug, Clone, PartialEq)]
pub struct RolledItem {
    pub class_name: String,
    pub name: String,
    pub category: Category,
    pub tier: u32,
    pub cost: u32,
    pub active: bool,
    /// The ability slot (1 to 4) an imbue item goes on.
    pub imbue_slot: Option<u8>,
    /// The hero's draft list marks the item "Good".
    pub recommended: bool,
}

/// One ability point spend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbilityStep {
    /// 1 to 4.
    pub slot: u8,
    /// 1 to 3: the ability's first, second or third upgrade.
    pub upgrade: u8,
    pub cost: u32,
    /// Ability points spent after this step.
    pub spent: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Roll {
    pub code: Code,
    pub hero_id: u32,
    pub hero_name: String,
    pub lane: Lane,
    /// In buying order: tier first, ties in random order.
    pub items: Vec<RolledItem>,
    /// Souls per category, weapon, vitality, spirit.
    pub souls: [u32; 3],
    pub abilities: Vec<AbilityStep>,
    /// The ability that reaches its third upgrade first.
    pub lead_ability: u8,
    /// Indices into the UI's rule list.
    pub rules: Vec<usize>,
    /// How many items the hero's draft list offered.
    pub pool: usize,
}

impl Roll {
    pub fn total(&self) -> u32 {
        self.souls.iter().sum()
    }

    /// The category with the most souls.
    pub fn focus(&self) -> Category {
        let mut best = Category::Weapon;
        for category in Category::ALL {
            if self.souls[category.index()] > self.souls[best.index()] {
                best = category;
            }
        }
        best
    }

    /// The build as plain text for a chat message, ending with the code.
    pub fn share_text(&self, data: &RollData) -> String {
        let mut out = format!("{} ({})\n", self.hero_name, self.lane.label());
        for (index, item) in self.items.iter().enumerate() {
            out.push_str(&format!("{:>2}. {} (T{} {})\n", index + 1, item.name, item.tier, item.category.label()));
        }
        if let Some(hero) = data.hero(self.hero_id) {
            let order: Vec<String> = self
                .abilities
                .iter()
                .map(|step| {
                    let name = data.ability_name(&hero.abilities[usize::from(step.slot - 1)]);
                    format!("{name} {}", step.upgrade)
                })
                .collect();
            out.push_str(&format!("Abilities: {}\n", order.join(", ")));
        }
        out.push_str(&format!("Sigil roll {}", self.code.text()));
        out
    }
}

/// Roll everything for `code`. Errors only when there is no hero to land on.
pub fn roll(data: &RollData, code: Code) -> Result<Roll> {
    let mut rng = Rng::new(code.seed);
    // 1. Hero. The draw happens even for a locked hero so the stream after it stays the same.
    let drawn = rng.below(data.heroes.len());
    let hero = code
        .hero
        .and_then(|id| data.hero(id))
        .or_else(|| data.heroes.get(drawn))
        .ok_or_else(|| Error::msg("There is no hero to roll."))?;
    // 2. Lane.
    let lane = if rng.below(2) == 0 { Lane::Solo } else { Lane::Duo };
    // 3. Build.
    let (items, pool) = roll_build(data, hero, &mut rng);
    let mut souls = [0u32; 3];
    for item in &items {
        souls[item.category.index()] += item.cost;
    }
    // 4. Abilities.
    let (abilities, lead_ability) = roll_abilities(&mut rng);
    // 5. Rules.
    let mut rules: Vec<usize> = (0..RULE_COUNT).collect();
    rng.shuffle(&mut rules);
    rules.truncate(RULES_DRAWN);
    Ok(Roll {
        code,
        hero_id: hero.id,
        hero_name: hero.name.clone(),
        lane,
        items,
        souls,
        abilities,
        lead_ability,
        rules,
        pool,
    })
}

fn roll_build(data: &RollData, hero: &RollHero, rng: &mut Rng) -> (Vec<RolledItem>, usize) {
    let drafted: Vec<&RollItem> = data
        .items
        .iter()
        .filter(|item| hero.draft.contains_key(&item.class_name))
        .collect();
    let pool: Vec<&RollItem> = if drafted.is_empty() {
        data.items.iter().collect()
    } else {
        drafted
    };
    let pool_size = pool.len();
    let max_tier = pool.iter().map(|item| item.tier).max().unwrap_or(1);
    let cheapest = pool.iter().map(|item| item.cost).min().unwrap_or(0);
    let mut left = BUDGET;
    let mut blocked: BTreeSet<&str> = BTreeSet::new();
    let mut picks: Vec<(RolledItem, f64)> = Vec::new();
    let total_picks = PER_CATEGORY * Category::ALL.len();
    let mut order = Category::ALL;
    rng.shuffle(&mut order);
    for category in order {
        let shape = SHAPES[rng.below(SHAPES.len())];
        let mut taken: HashMap<u32, u32> = HashMap::new();
        for wanted in shape {
            let picks_left = total_picks - picks.len();
            let reserve = (picks_left.saturating_sub(1) as u32).saturating_mul(cheapest);
            let spendable = left.saturating_sub(reserve);
            let mut chosen: Option<&RollItem> = None;
            for tier in nearest_tiers(wanted, max_tier) {
                let cap = hero
                    .tier_caps
                    .get(&category)
                    .and_then(|caps| caps.get(tier as usize - 1))
                    .copied()
                    .unwrap_or(PER_CATEGORY as u32);
                if taken.get(&tier).copied().unwrap_or(0) >= cap {
                    continue;
                }
                let candidates: Vec<&RollItem> = pool
                    .iter()
                    .copied()
                    .filter(|item| item.category == category && item.tier == tier)
                    .filter(|item| !blocked.contains(item.class_name.as_str()))
                    .filter(|item| item.cost <= spendable)
                    .collect();
                let weights: Vec<f64> = candidates
                    .iter()
                    .map(|item| match hero.draft.get(&item.class_name) {
                        Some((true, weight)) => weight * 3.0,
                        Some((false, weight)) => *weight,
                        None => 1.0,
                    })
                    .collect();
                if let Some(index) = rng.weighted(&weights) {
                    chosen = Some(candidates[index]);
                    break;
                }
            }
            let Some(item) = chosen else { continue };
            *taken.entry(item.tier).or_default() += 1;
            left -= item.cost;
            blocked.insert(item.class_name.as_str());
            for component in &item.components {
                blocked.insert(component.as_str());
            }
            for other in pool.iter() {
                if other.components.iter().any(|component| component == &item.class_name) {
                    blocked.insert(other.class_name.as_str());
                }
            }
            let imbue_slot = item.imbue.as_deref().map(|imbue| {
                let slots = if imbue == "imbue_active_non_ult" { 3 } else { 4 };
                (rng.below(slots) + 1) as u8
            });
            let recommended = hero.draft.get(&item.class_name).is_some_and(|(good, _)| *good);
            picks.push((
                RolledItem {
                    class_name: item.class_name.clone(),
                    name: item.name.clone(),
                    category,
                    tier: item.tier,
                    cost: item.cost,
                    active: item.active,
                    imbue_slot,
                    recommended,
                },
                0.0,
            ));
        }
    }
    for pick in &mut picks {
        pick.1 = rng.unit();
    }
    picks.sort_by(|a, b| a.0.tier.cmp(&b.0.tier).then(a.1.total_cmp(&b.1)));
    (picks.into_iter().map(|(item, _)| item).collect(), pool_size)
}

/// `wanted`, then one below, one above, two below and so on, inside `1..=max`.
fn nearest_tiers(wanted: u32, max: u32) -> Vec<u32> {
    let wanted = wanted.clamp(1, max.max(1)) as i64;
    let mut out = vec![wanted as u32];
    for distance in 1..=max as i64 {
        for tier in [wanted - distance, wanted + distance] {
            if tier >= 1 && tier <= max as i64 {
                out.push(tier as u32);
            }
        }
    }
    out
}

fn roll_abilities(rng: &mut Rng) -> (Vec<AbilityStep>, u8) {
    let mut level = [0usize; 4];
    let mut spent = 0u32;
    let mut steps = Vec::with_capacity(12);
    let mut lead = 0u8;
    loop {
        let weights: Vec<f64> = level
            .iter()
            .map(|level| AP_COSTS.get(*level).map(|cost| 1.0 / f64::from(*cost)).unwrap_or(0.0))
            .collect();
        let Some(slot) = rng.weighted(&weights) else { break };
        let cost = AP_COSTS[level[slot]];
        level[slot] += 1;
        spent += cost;
        steps.push(AbilityStep {
            slot: slot as u8 + 1,
            upgrade: level[slot] as u8,
            cost,
            spent,
        });
        if level[slot] == AP_COSTS.len() && lead == 0 {
            lead = slot as u8 + 1;
        }
    }
    (steps, lead.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(class_name: &str, category: &str, tier: u32, cost: u32, components: &[&str]) -> Value {
        serde_json::json!({
            "id": 1, "class_name": class_name, "name": class_name.to_uppercase(),
            "item_slot_type": category, "item_tier": tier, "cost": cost, "shopable": true,
            "is_active_item": false, "imbue": null, "component_items": components,
        })
    }

    fn data() -> RollData {
        let mut items = Vec::new();
        for category in ["weapon", "vitality", "spirit"] {
            for (tier, cost) in [(1, 800), (2, 1600), (3, 3200), (4, 6400)] {
                for n in 0..4 {
                    items.push(item(&format!("{category}_{tier}_{n}"), category, tier, cost, &[]));
                }
            }
        }
        // An upgrade of weapon_1_0, an unreleased item and an unbuyable one.
        items.push(item("weapon_3_up", "weapon", 3, 3200, &["weapon_1_0"]));
        items.push(item("weapon_x", "weapon", 2, 9999, &[]));
        let mut hidden = item("weapon_hidden", "weapon", 2, 1600, &[]);
        hidden["shopable"] = Value::Bool(false);
        items.push(hidden);
        let hero = |id: u32, name: &str, selectable: bool| {
            serde_json::json!({
                "id": id, "class_name": format!("hero_{id}"), "name": name,
                "player_selectable": selectable, "disabled": false, "in_development": true,
                "complexity": 2, "hero_type": "marksman", "gun_tag": "Rapid", "tags": ["A", "A", "B"],
                "description": {"role": "Shoots"},
                "items": {"signature1": "ability_a", "signature2": "ability_b", "signature3": "ability_c", "signature4": "ability_d"},
                "item_slot_info": {"weapon": {"max_purchases_for_tier": [6, 6, 6, 6]}},
                "colors": {"ui": [1, 2, 3]},
            })
        };
        let heroes = serde_json::json!([hero(1, "One", true), hero(2, "Two", true), hero(3, "Gone", false)]);
        RollData {
            heroes: parse_heroes(&heroes.to_string()).unwrap(),
            items: parse_items(&Value::Array(items).to_string()).unwrap(),
            abilities: parse_abilities(r#"[{"class_name":"ability_a","name":"Blast"}]"#).unwrap(),
        }
    }

    #[test]
    fn parsing_keeps_playable_heroes_and_buyable_items() {
        let data = data();
        assert_eq!(data.heroes.iter().map(|hero| hero.id).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(data.heroes[0].tags, vec!["Rapid", "A", "B"]);
        assert_eq!(data.heroes[0].accent, [1, 2, 3]);
        assert!(data.items.iter().all(|item| item.class_name != "weapon_x" && item.class_name != "weapon_hidden"));
        assert_eq!(data.ability_name("ability_a"), "Blast");
        assert_eq!(data.ability_name("ability_flame_dash"), "Flame Dash");
    }

    #[test]
    fn the_same_code_rolls_the_same_build() {
        let data = data();
        let code = Code { seed: 123_456, hero: None };
        assert_eq!(roll(&data, code).unwrap(), roll(&data, code).unwrap());
        let other = roll(&data, Code { seed: 123_457, hero: None }).unwrap();
        assert_ne!(roll(&data, code).unwrap().items, other.items);
    }

    #[test]
    fn a_build_keeps_the_slot_and_budget_rules() {
        let data = data();
        for seed in 1..400 {
            let roll = roll(&data, Code { seed, hero: None }).unwrap();
            assert!(roll.total() <= BUDGET, "seed {seed} spends {}", roll.total());
            assert_eq!(roll.items.len(), 12, "seed {seed}");
            for category in Category::ALL {
                assert_eq!(roll.items.iter().filter(|item| item.category == category).count(), 4);
            }
            let names: BTreeSet<&str> = roll.items.iter().map(|item| item.class_name.as_str()).collect();
            assert_eq!(names.len(), 12, "no duplicates");
            assert!(!(names.contains("weapon_3_up") && names.contains("weapon_1_0")), "no component overlap");
            assert!(roll.items.windows(2).all(|pair| pair[0].tier <= pair[1].tier), "buying order");
            assert_eq!(roll.abilities.len(), 12);
            assert_eq!(roll.abilities.last().unwrap().spent, 32);
            assert_eq!(roll.rules.len(), RULES_DRAWN);
            assert!(roll.rules.iter().all(|rule| *rule < RULE_COUNT));
        }
    }

    #[test]
    fn a_locked_hero_keeps_the_rest_of_the_stream() {
        let data = data();
        let free = roll(&data, Code { seed: 99, hero: None }).unwrap();
        let locked = roll(&data, Code { seed: 99, hero: Some(free.hero_id) }).unwrap();
        assert_eq!(free, Roll { code: free.code, ..locked.clone() });
        let other = if free.hero_id == 1 { 2 } else { 1 };
        assert_eq!(roll(&data, Code { seed: 99, hero: Some(other) }).unwrap().hero_id, other);
    }

    #[test]
    fn codes_round_trip_and_are_found_in_text() {
        for code in [Code { seed: 0, hero: None }, Code { seed: u32::MAX, hero: Some(12) }, Code { seed: 777, hero: None }] {
            assert_eq!(Code::find(&code.text()), Some(code));
        }
        assert_eq!(
            Code::find("try this: dlr-lfls-h7 good luck"),
            Some(Code { seed: u32::from_str_radix("LFLS", 36).unwrap(), hero: Some(7) })
        );
        assert_eq!(Code::find("no code"), None);
        assert_eq!(Code::find("DLR-"), None);
    }

    #[test]
    fn tiers_are_tried_nearest_first() {
        assert_eq!(nearest_tiers(2, 4), vec![2, 1, 3, 4]);
        assert_eq!(nearest_tiers(4, 4), vec![4, 3, 2, 1]);
        assert_eq!(nearest_tiers(5, 3), vec![3, 2, 1]);
    }

    #[test]
    fn the_share_text_names_hero_items_and_code() {
        let data = data();
        let roll = roll(&data, Code { seed: 5, hero: None }).unwrap();
        let text = roll.share_text(&data);
        assert!(text.starts_with(&roll.hero_name));
        assert!(text.ends_with(&roll.code.text()));
        assert_eq!(text.lines().filter(|line| line.contains(". ")).count(), 12);
    }
}
