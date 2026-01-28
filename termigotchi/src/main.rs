use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, Clear, ClearType, DisableLineWrap, EnableLineWrap,
        EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    cmp::{max, min},
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const SAVE_VERSION: u32 = 1;
const GAME_VERSION: u32 = 1;

fn main() -> Result<()> {
    let mut app = App::init()?;
    app.run()?;
    Ok(())
}

/* -----------------------------
   Settings + Save paths
------------------------------ */

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Settings {
    skin_name: String,
    fps_cap: u32,
    enable_color: bool,
    enable_braille: bool,
    seed: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            skin_name: "default".to_string(),
            fps_cap: 30,
            enable_color: true,
            enable_braille: true,
            seed: 0xC0FFEE_u64,
        }
    }
}

struct Paths {
    dir: PathBuf,
    save_path: PathBuf,
    settings_path: PathBuf,
}

fn project_paths() -> Result<Paths> {
    let proj = ProjectDirs::from("com", "termigotchi", "Termigotchi")
        .context("could not resolve project directories")?;
    let dir = proj.data_local_dir().to_path_buf();
    fs::create_dir_all(&dir).ok();
    Ok(Paths {
        save_path: dir.join("save.json"),
        settings_path: dir.join("settings.json"),
        dir,
    })
}

fn load_settings(path: &Path) -> Settings {
    if let Ok(s) = fs::read_to_string(path) {
        if let Ok(v) = serde_json::from_str::<Settings>(&s) {
            return v;
        }
    }
    Settings::default()
}

fn save_settings_atomic(path: &Path, s: &Settings) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(s)?;
    fs::write(&tmp, data)?;
    atomic_rename(&tmp, path)?;
    Ok(())
}

fn atomic_rename(from: &Path, to: &Path) -> Result<()> {
    // Best-effort atomic replace on same filesystem.
    // On Windows, rename-over-existing is trickier; this is still fine for Linux server usage.
    if to.exists() {
        let _ = fs::remove_file(to);
    }
    fs::rename(from, to)?;
    Ok(())
}

/* -----------------------------
   Data Model (trimmed but faithful)
------------------------------ */

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Scene {
    Main,
    Settings,
    Help,
    Rename,
    Recap(CatchupSummary),
    Dead,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
enum LifeStage {
    Egg,
    Baby,
    Child,
    Teen,
    Adult,
    Elder,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
enum Mood {
    Happy,
    Okay,
    Sad,
    Angry,
    Sick,
    Sleepy,
    Bored,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Meters {
    hunger: f32,
    happiness: f32,
    health: f32,
    energy: f32,
    hygiene: f32,
    discipline: f32,
    bond: f32,
    weight: f32,
}

impl Default for Meters {
    fn default() -> Self {
        Self {
            hunger: 80.0,
            happiness: 70.0,
            health: 90.0,
            energy: 80.0,
            hygiene: 85.0,
            discipline: 30.0,
            bond: 15.0,
            weight: 20.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Flags {
    sleeping: bool,
    sick: bool,
    dirty: bool,
    attention_call: bool,
    has_poop: bool,
    dead: bool,
}

impl Default for Flags {
    fn default() -> Self {
        Self {
            sleeping: false,
            sick: false,
            dirty: false,
            attention_call: false,
            has_poop: false,
            dead: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct History {
    neglected_events: u32,
    play_events: u32,
    feed_events: u32,
    clean_events: u32,
    discipline_events: u32,
    sickness_events: u32,
}

impl Default for History {
    fn default() -> Self {
        Self {
            neglected_events: 0,
            play_events: 0,
            feed_events: 0,
            clean_events: 0,
            discipline_events: 0,
            sickness_events: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Pet {
    name: String,
    species_id: String,
    stage: LifeStage,
    age_ticks: u64,
    mood: Mood,
    meters: Meters,
    flags: Flags,
    history: History,
}

impl Pet {
    fn new_default() -> Self {
        Self {
            name: "Mochi".to_string(),
            species_id: "default".to_string(),
            stage: LifeStage::Egg,
            age_ticks: 0,
            mood: Mood::Okay,
            meters: Meters::default(),
            flags: Flags::default(),
            history: History::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RNGState {
    seed: u64,
    event_counter: u64,
}

impl RNGState {
    fn new(seed: u64) -> Self {
        Self {
            seed,
            event_counter: 0,
        }
    }

    fn next_u64(&mut self) -> u64 {
        // Counter-based SplitMix64: deterministic and cheap.
        let mut z = self
            .seed
            .wrapping_add(self.event_counter.wrapping_mul(0x9E3779B97F4A7C15));
        self.event_counter = self.event_counter.wrapping_add(1);

        z = z.wrapping_add(0x9E3779B97F4A7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    fn next_f32(&mut self) -> f32 {
        // [0,1)
        let v = self.next_u64() >> 40; // 24 bits
        (v as f32) / ((1u64 << 24) as f32)
    }

    fn roll(&mut self, p: f32) -> bool {
        self.next_f32() < p.clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct DecayRates {
    hunger: f32,
    happiness: f32,
    energy: f32,
    hygiene: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Rules {
    tick_step_ms: u64,     // 250ms typical
    catchup_step_ms: u64,  // 5000ms typical
    catchup_max_secs: i64, // 7 days typical
    meter_rate_scale: f32, // 1.0 = normal, lower = slower stat changes
    decay_by_stage: BTreeMap<LifeStage, DecayRates>,
}

impl Default for Rules {
    fn default() -> Self {
        let mut decay_by_stage = BTreeMap::new();
        decay_by_stage.insert(
            LifeStage::Egg,
            DecayRates {
                hunger: 1.0,
                happiness: 0.6,
                energy: 0.4,
                hygiene: 0.5,
            },
        );
        decay_by_stage.insert(
            LifeStage::Baby,
            DecayRates {
                hunger: 1.3,
                happiness: 0.9,
                energy: 0.8,
                hygiene: 0.7,
            },
        );
        decay_by_stage.insert(
            LifeStage::Child,
            DecayRates {
                hunger: 1.0,
                happiness: 0.8,
                energy: 0.7,
                hygiene: 0.6,
            },
        );
        decay_by_stage.insert(
            LifeStage::Teen,
            DecayRates {
                hunger: 0.9,
                happiness: 0.9,
                energy: 0.8,
                hygiene: 0.7,
            },
        );
        decay_by_stage.insert(
            LifeStage::Adult,
            DecayRates {
                hunger: 0.8,
                happiness: 0.6,
                energy: 0.6,
                hygiene: 0.6,
            },
        );
        decay_by_stage.insert(
            LifeStage::Elder,
            DecayRates {
                hunger: 0.7,
                happiness: 0.6,
                energy: 0.7,
                hygiene: 0.7,
            },
        );

        Self {
            tick_step_ms: 500,
            catchup_step_ms: 5000,
            catchup_max_secs: 7 * 24 * 3600,
            meter_rate_scale: 0.02,
            decay_by_stage,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GameState {
    version: u32,
    pet: Pet,
    scene: Scene,
    rng: RNGState,
    sim_ticks: u64,
    last_action_at_tick: u64,
    #[serde(default)]
    settings_cursor: usize,
    #[serde(default)]
    name_edit: String,
}

impl GameState {
    fn new(seed: u64) -> Self {
        Self {
            version: GAME_VERSION,
            pet: Pet::new_default(),
            scene: Scene::Main,
            rng: RNGState::new(seed),
            sim_ticks: 0,
            last_action_at_tick: 0,
            settings_cursor: 0,
            name_edit: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SaveFile {
    version: u32,
    last_seen_utc: DateTime<Utc>,
    state: GameState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CatchupSummary {
    ticks_simulated: u64,
    became_sick: bool,
    attention_calls: u32,
    died: bool,
    hunger_min: f32,
    happiness_min: f32,
    health_min: f32,
}

impl CatchupSummary {
    fn new() -> Self {
        Self {
            ticks_simulated: 0,
            became_sick: false,
            attention_calls: 0,
            died: false,
            hunger_min: 100.0,
            happiness_min: 100.0,
            health_min: 100.0,
        }
    }

    fn has_anything(&self) -> bool {
        self.ticks_simulated > 0
            && (self.became_sick
                || self.attention_calls > 0
                || self.died
                || self.hunger_min < 40.0
                || self.happiness_min < 40.0
                || self.health_min < 60.0)
    }

    fn record(&mut self, st: &GameState) {
        self.ticks_simulated += 1;
        self.hunger_min = self.hunger_min.min(st.pet.meters.hunger);
        self.happiness_min = self.happiness_min.min(st.pet.meters.happiness);
        self.health_min = self.health_min.min(st.pet.meters.health);
        if st.pet.flags.sick {
            self.became_sick = true;
        }
        if st.pet.flags.attention_call {
            self.attention_calls += 1;
        }
        if st.pet.flags.dead {
            self.died = true;
        }
    }
}

/* -----------------------------
   Actions / Input mapping (minimal but expandable)
------------------------------ */

#[derive(Clone, Debug)]
enum PlayerAction {
    Feed(&'static str),
    PlayAny,
    Clean,
    Medicine,
    SleepToggle,
    Discipline,
    DebugKill,
    HelpToggle,
    RenameOpen,
    RenameChar(char),
    RenameBackspace,
    RenameCommit,
    RenameCancel,
    SettingsMove(i32),
    SettingsToggle,
    SettingsOpen,
    Back,
    Quit,
    NewGame,
}

impl GameState {
    fn apply(&mut self, action: PlayerAction) {
        match action {
            PlayerAction::Feed(id) => {
                let _ = id;
                self.pet.meters.hunger = (self.pet.meters.hunger + 18.0).clamp(0.0, 100.0);
                self.pet.meters.happiness = (self.pet.meters.happiness + 3.0).clamp(0.0, 100.0);
                self.pet.meters.weight = (self.pet.meters.weight + 0.6).clamp(0.0, 200.0);
                self.pet.history.feed_events += 1;
                self.last_action_at_tick = self.sim_ticks;

                // deterministic delayed poop chance: just roll now for skeleton
                if self.rng.roll(0.25) {
                    self.pet.flags.has_poop = true;
                }
            }
            PlayerAction::PlayAny => {
                self.pet.meters.happiness = (self.pet.meters.happiness + 10.0).clamp(0.0, 100.0);
                self.pet.meters.energy = (self.pet.meters.energy - 7.0).clamp(0.0, 100.0);
                self.pet.meters.hunger = (self.pet.meters.hunger - 1.5).clamp(0.0, 100.0);
                self.pet.history.play_events += 1;
                self.last_action_at_tick = self.sim_ticks;
            }
            PlayerAction::Clean => {
                self.pet.meters.hygiene = 100.0;
                self.pet.flags.dirty = false;
                self.pet.flags.has_poop = false;
                self.pet.history.clean_events += 1;
                self.last_action_at_tick = self.sim_ticks;
            }
            PlayerAction::Medicine => {
                if self.pet.flags.sick {
                    self.pet.flags.sick = false;
                    self.pet.meters.health = (self.pet.meters.health + 22.0).clamp(0.0, 100.0);
                    self.last_action_at_tick = self.sim_ticks;
                }
            }
            PlayerAction::SleepToggle => {
                self.pet.flags.sleeping = !self.pet.flags.sleeping;
                self.last_action_at_tick = self.sim_ticks;
            }
            PlayerAction::Discipline => {
                self.pet.meters.discipline = (self.pet.meters.discipline + 12.0).clamp(0.0, 100.0);
                self.pet.flags.attention_call = false;
                self.pet.history.discipline_events += 1;
                self.last_action_at_tick = self.sim_ticks;
            }
            PlayerAction::DebugKill => {
                self.pet.flags.dead = true;
                self.scene = Scene::Dead;
            }
            PlayerAction::HelpToggle => {
                self.scene = match self.scene {
                    Scene::Help => Scene::Main,
                    _ => Scene::Help,
                };
            }
            PlayerAction::RenameOpen => {
                self.name_edit = self.pet.name.clone();
                self.scene = Scene::Rename;
            }
            PlayerAction::RenameChar(ch) => {
                const NAME_MAX: usize = 18;
                if self.name_edit.len() < NAME_MAX {
                    self.name_edit.push(ch);
                }
            }
            PlayerAction::RenameBackspace => {
                self.name_edit.pop();
            }
            PlayerAction::RenameCommit => {
                let trimmed = self.name_edit.trim();
                if !trimmed.is_empty() {
                    self.pet.name = trimmed.to_string();
                }
                self.scene = Scene::Settings;
            }
            PlayerAction::RenameCancel => {
                self.scene = Scene::Settings;
            }
            PlayerAction::SettingsMove(delta) => {
                let len = 2i32;
                let mut next = self.settings_cursor as i32 + delta;
                if next < 0 {
                    next = len - 1;
                } else if next >= len {
                    next = 0;
                }
                self.settings_cursor = next as usize;
            }
            PlayerAction::SettingsToggle => {}
            PlayerAction::SettingsOpen => {
                self.scene = Scene::Settings;
                self.settings_cursor = 0;
            }
            PlayerAction::Back => self.scene = Scene::Main,
            PlayerAction::Quit => {}
            PlayerAction::NewGame => {
                let seed = self.rng.seed;
                *self = GameState::new(seed);
            }
        }
    }

    fn tick_fixed_step(&mut self, rules: &Rules) {
        if self.pet.flags.dead {
            return;
        }

        self.sim_ticks += 1;
        self.pet.age_ticks += 1;

        let dt = (rules.tick_step_ms as f32 / 1000.0) * rules.meter_rate_scale;

        let decay = rules
            .decay_by_stage
            .get(&self.pet.stage)
            .copied()
            .unwrap_or(DecayRates {
                hunger: 1.0,
                happiness: 0.8,
                energy: 0.7,
                hygiene: 0.6,
            });

        self.pet.meters.hunger = (self.pet.meters.hunger - decay.hunger * dt).clamp(0.0, 100.0);
        self.pet.meters.happiness =
            (self.pet.meters.happiness - decay.happiness * dt).clamp(0.0, 100.0);

        if !self.pet.flags.sleeping {
            self.pet.meters.energy = (self.pet.meters.energy - decay.energy * dt).clamp(0.0, 100.0);
        } else {
            let recover = 9.0;
            self.pet.meters.energy = (self.pet.meters.energy + recover * dt).clamp(0.0, 100.0);
            let sleep_hunger = 0.7;
            self.pet.meters.hunger = (self.pet.meters.hunger - sleep_hunger * dt).clamp(0.0, 100.0);
            if self.pet.meters.energy >= 99.5 && self.rng.roll(0.02) {
                self.pet.flags.sleeping = false;
            }
        }

        self.pet.meters.hygiene = (self.pet.meters.hygiene - decay.hygiene * dt).clamp(0.0, 100.0);

        if self.pet.flags.has_poop {
            let poop_penalty = 2.0;
            self.pet.meters.hygiene =
                (self.pet.meters.hygiene - poop_penalty * dt).clamp(0.0, 100.0);
            self.pet.flags.dirty = true;
        }

        // sickness chance (simple)
        let mut sick_chance = 0.0;
        if self.pet.meters.hygiene < 20.0 {
            sick_chance += 0.0025;
        }
        if self.pet.meters.hunger < 10.0 {
            sick_chance += 0.0035;
        }
        sick_chance *= rules.meter_rate_scale;
        if !self.pet.flags.sick && self.rng.roll(sick_chance) {
            self.pet.flags.sick = true;
            self.pet.history.sickness_events += 1;
        }
        if self.pet.flags.sick {
            self.pet.meters.health = (self.pet.meters.health - 2.6 * dt).clamp(0.0, 100.0);
            self.pet.meters.happiness = (self.pet.meters.happiness - 1.6 * dt).clamp(0.0, 100.0);
        } else {
            // tiny passive recovery
            self.pet.meters.health = (self.pet.meters.health + 0.25 * dt).clamp(0.0, 100.0);
        }

        // attention calls
        let needs_attention = self.pet.meters.hunger < 25.0 || self.pet.meters.happiness < 25.0;
        let cooldown_ticks = (10_000u64 / rules.tick_step_ms).max(1); // 10s
        if needs_attention
            && (self.sim_ticks.saturating_sub(self.last_action_at_tick) > cooldown_ticks)
            && self.rng.roll(0.01)
        {
            self.pet.flags.attention_call = true;
        }

        self.pet.mood = derive_mood(&self.pet.meters, &self.pet.flags);

        // evolution (very simple thresholds)
        self.maybe_evolve(rules);

        // neglect / death
        if self.pet.meters.health <= 0.1 {
            self.pet.flags.dead = true;
        }

        // stage-specific slow weight drift
        if self.pet.meters.hunger < 10.0 && self.rng.roll(0.01) {
            self.pet.meters.weight = (self.pet.meters.weight - 0.2).clamp(0.0, 200.0);
        }
    }

    fn maybe_evolve(&mut self, rules: &Rules) {
        let ticks = self.pet.age_ticks;

        let t_egg = (20_000u64 / rules.tick_step_ms).max(1); // ~20s
        let t_baby = (90_000u64 / rules.tick_step_ms).max(1); // ~90s
        let t_child = (240_000u64 / rules.tick_step_ms).max(1); // ~4m
        let t_teen = (480_000u64 / rules.tick_step_ms).max(1); // ~8m

        let next = match self.pet.stage {
            LifeStage::Egg if ticks >= t_egg => Some(LifeStage::Baby),
            LifeStage::Baby if ticks >= t_baby => Some(LifeStage::Child),
            LifeStage::Child if ticks >= t_child => Some(LifeStage::Teen),
            LifeStage::Teen if ticks >= t_teen => Some(LifeStage::Adult),
            _ => None,
        };

        if let Some(ns) = next {
            self.pet.stage = ns;
            // small stage bonus
            self.pet.meters.happiness = (self.pet.meters.happiness + 8.0).clamp(0.0, 100.0);
            self.pet.meters.health = (self.pet.meters.health + 8.0).clamp(0.0, 100.0);
        }
    }
}

fn derive_mood(m: &Meters, f: &Flags) -> Mood {
    if f.dead {
        return Mood::Sad;
    }
    if f.sick {
        return Mood::Sick;
    }
    if f.sleeping {
        return Mood::Sleepy;
    }
    if m.hunger < 15.0 || m.happiness < 15.0 {
        return Mood::Angry;
    }
    if m.happiness > 70.0 && m.hunger > 55.0 && m.health > 70.0 {
        return Mood::Happy;
    }
    if m.happiness < 35.0 {
        return Mood::Bored;
    }
    Mood::Okay
}

/* -----------------------------
   Offline catch-up
------------------------------ */

fn catch_up(
    state: &mut GameState,
    last_seen: DateTime<Utc>,
    now: DateTime<Utc>,
    rules: &Rules,
) -> CatchupSummary {
    let elapsed = now - last_seen;
    let max_elapsed = ChronoDuration::seconds(rules.catchup_max_secs.max(0));
    let elapsed = elapsed.clamp(ChronoDuration::zero(), max_elapsed);

    let mut summary = CatchupSummary::new();

    let tick_step = ChronoDuration::milliseconds(rules.tick_step_ms as i64);
    let catch_step = ChronoDuration::milliseconds(rules.catchup_step_ms as i64);

    let mut remaining = elapsed;

    while remaining > ChronoDuration::zero() && !state.pet.flags.dead {
        let step = if remaining < catch_step {
            remaining
        } else {
            catch_step
        };
        let ticks = (step.num_milliseconds() / tick_step.num_milliseconds()).max(0) as u64;
        if ticks == 0 {
            break;
        }
        for _ in 0..ticks {
            state.tick_fixed_step(rules);
            summary.record(state);
            if state.pet.flags.dead {
                break;
            }
        }
        remaining =
            remaining - ChronoDuration::milliseconds((ticks as i64) * tick_step.num_milliseconds());
    }

    summary
}

/* -----------------------------
   Terminal backend + buffers + braille encoding
------------------------------ */

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cell {
    ch: char,
    fg: Color,
    bg: Color,
    bold: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::White,
            bg: Color::Black,
            bold: false,
        }
    }
}

struct CellBuffer {
    w: u16,
    h: u16,
    cells: Vec<Cell>,
}

impl CellBuffer {
    fn new(w: u16, h: u16) -> Self {
        Self {
            w,
            h,
            cells: vec![Cell::default(); (w as usize) * (h as usize)],
        }
    }
    fn idx(&self, x: u16, y: u16) -> usize {
        (y as usize) * (self.w as usize) + (x as usize)
    }
    fn set(&mut self, x: u16, y: u16, c: Cell) {
        if x < self.w && y < self.h {
            let i = self.idx(x, y);
            self.cells[i] = c;
        }
    }
    fn get(&self, x: u16, y: u16) -> Cell {
        if x < self.w && y < self.h {
            self.cells[self.idx(x, y)]
        } else {
            Cell::default()
        }
    }
    fn clear(&mut self, bg: Color) {
        for c in &mut self.cells {
            c.ch = ' ';
            c.fg = Color::White;
            c.bg = bg;
            c.bold = false;
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Pixel {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

struct PixelCanvas {
    w: u32,
    h: u32,
    px: Vec<Pixel>,
}

impl PixelCanvas {
    fn new(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            px: vec![Pixel::default(); (w as usize) * (h as usize)],
        }
    }
    fn idx(&self, x: u32, y: u32) -> usize {
        (y as usize) * (self.w as usize) + (x as usize)
    }
    fn clear(&mut self, p: Pixel) {
        self.px.fill(p);
    }
    fn blend_over(&mut self, x: i32, y: i32, src: Pixel) {
        if x < 0 || y < 0 {
            return;
        }
        let (x, y) = (x as u32, y as u32);
        if x >= self.w || y >= self.h {
            return;
        }
        let i = self.idx(x, y);
        let dst = self.px[i];

        let sa = src.a as f32 / 255.0;
        let da = dst.a as f32 / 255.0;

        let out_a = sa + da * (1.0 - sa);
        if out_a <= 1e-6 {
            self.px[i] = Pixel::default();
            return;
        }

        let blend = |sc: u8, dc: u8| -> u8 {
            let sc = sc as f32 / 255.0;
            let dc = dc as f32 / 255.0;
            let out = (sc * sa + dc * da * (1.0 - sa)) / out_a;
            (out.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
        };

        self.px[i] = Pixel {
            r: blend(src.r, dst.r),
            g: blend(src.g, dst.g),
            b: blend(src.b, dst.b),
            a: (out_a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        };
    }
}

struct Terminal {
    out: io::Stdout,
    cols: u16,
    rows: u16,
    prev: CellBuffer,
    cur: CellBuffer,
    canvas: PixelCanvas,
}

impl Terminal {
    fn begin() -> Result<Self> {
        let mut out = io::stdout();
        execute!(
            out,
            EnterAlternateScreen,
            cursor::Hide,
            DisableLineWrap,
            terminal::Clear(ClearType::All)
        )?;
        terminal::enable_raw_mode()?;

        let (cols, rows) = terminal::size()?;
        let prev = CellBuffer::new(cols, rows);
        let cur = CellBuffer::new(cols, rows);

        // Braille: 2×4 pixels per cell
        let canvas = PixelCanvas::new(cols as u32 * 2, rows as u32 * 4);

        Ok(Self {
            out,
            cols,
            rows,
            prev,
            cur,
            canvas,
        })
    }

    fn end(&mut self) -> Result<()> {
        queue!(
            self.out,
            BeginSynchronizedUpdate,
            ResetColor,
            Clear(ClearType::All),
            cursor::Show,
            EnableLineWrap,
            EndSynchronizedUpdate,
            LeaveAlternateScreen
        )?;
        self.out.flush()?;
        terminal::disable_raw_mode()?;
        Ok(())
    }

    fn resize_if_needed(&mut self) -> Result<bool> {
        let (c, r) = terminal::size()?;
        if c == self.cols && r == self.rows {
            return Ok(false);
        }
        self.cols = c;
        self.rows = r;
        self.prev = CellBuffer::new(c, r);
        self.cur = CellBuffer::new(c, r);
        self.canvas = PixelCanvas::new(c as u32 * 2, r as u32 * 4);
        Ok(true)
    }

    fn present(&mut self, diff_only: bool) -> Result<()> {
        queue!(self.out, BeginSynchronizedUpdate)?;

        let mut last_fg = None;
        let mut last_bg = None;

        for y in 0..self.rows {
            for x in 0..self.cols {
                let i = self.cur.idx(x, y);
                let c = self.cur.cells[i];
                if diff_only && c == self.prev.cells[i] {
                    continue;
                }

                queue!(self.out, cursor::MoveTo(x, y))?;

                if last_fg != Some(c.fg) {
                    queue!(self.out, SetForegroundColor(c.fg))?;
                    last_fg = Some(c.fg);
                }
                if last_bg != Some(c.bg) {
                    queue!(self.out, SetBackgroundColor(c.bg))?;
                    last_bg = Some(c.bg);
                }

                queue!(self.out, Print(c.ch))?;
            }
        }

        queue!(self.out, ResetColor, EndSynchronizedUpdate)?;
        self.out.flush()?;
        self.prev.cells.copy_from_slice(&self.cur.cells);
        Ok(())
    }
}

/* -----------------------------
   Braille encoding: 2×4 pixels -> U+2800..U+28FF
------------------------------ */

fn braille_bit(dx: u32, dy: u32) -> u8 {
    // Dot mapping:
    // (0,0)=1 (0,1)=2 (0,2)=4 (0,3)=64
    // (1,0)=8 (1,1)=16 (1,2)=32 (1,3)=128
    match (dx, dy) {
        (0, 0) => 0x01,
        (0, 1) => 0x02,
        (0, 2) => 0x04,
        (0, 3) => 0x40,
        (1, 0) => 0x08,
        (1, 1) => 0x10,
        (1, 2) => 0x20,
        (1, 3) => 0x80,
        _ => 0x00,
    }
}

fn canvas_to_cells(canvas: &PixelCanvas, out: &mut CellBuffer, enable_color: bool, bg: Color) {
    let cols = out.w as u32;
    let rows = out.h as u32;

    for cy in 0..rows {
        for cx in 0..cols {
            let px0 = cx * 2;
            let py0 = cy * 4;

            let mut mask: u8 = 0;
            let mut sum_r: u32 = 0;
            let mut sum_g: u32 = 0;
            let mut sum_b: u32 = 0;
            let mut ink_count: u32 = 0;

            for dy in 0..4 {
                for dx in 0..2 {
                    let x = px0 + dx;
                    let y = py0 + dy;
                    if x >= canvas.w || y >= canvas.h {
                        continue;
                    }
                    let p = canvas.px[canvas.idx(x, y)];
                    let a = p.a as u32;

                    // threshold: treat alpha as ink
                    if a >= 32 {
                        mask |= braille_bit(dx, dy);
                        sum_r += p.r as u32;
                        sum_g += p.g as u32;
                        sum_b += p.b as u32;
                        ink_count += 1;
                    }
                }
            }

            let ch = char::from_u32(0x2800 + (mask as u32)).unwrap_or(' ');

            let fg = if enable_color && ink_count > 0 {
                let r = (sum_r / ink_count) as u8;
                let g = (sum_g / ink_count) as u8;
                let b = (sum_b / ink_count) as u8;
                Color::Rgb { r, g, b }
            } else {
                Color::White
            };

            out.set(
                cx as u16,
                cy as u16,
                Cell {
                    ch,
                    fg,
                    bg,
                    bold: false,
                },
            );
        }
    }
}

/* -----------------------------
   Layered skin renderer (anchors + z-order)
------------------------------ */

struct Renderer;

impl Renderer {
    fn draw_pet(canvas: &mut PixelCanvas, st: &GameState, viewport: Viewport, offset: (i32, i32)) {
        let center_x = viewport.x + viewport.w / 2 + offset.0;
        let center_y = viewport.y + viewport.h / 2 + offset.1;

        let mood_col = match st.pet.mood {
            Mood::Happy => Pixel {
                r: 140,
                g: 240,
                b: 200,
                a: 220,
            },
            Mood::Okay => Pixel {
                r: 150,
                g: 170,
                b: 240,
                a: 210,
            },
            Mood::Sad => Pixel {
                r: 130,
                g: 150,
                b: 220,
                a: 200,
            },
            Mood::Angry => Pixel {
                r: 255,
                g: 90,
                b: 90,
                a: 225,
            },
            Mood::Sick => Pixel {
                r: 140,
                g: 240,
                b: 130,
                a: 220,
            },
            Mood::Sleepy => Pixel {
                r: 160,
                g: 140,
                b: 255,
                a: 210,
            },
            Mood::Bored => Pixel {
                r: 200,
                g: 200,
                b: 200,
                a: 205,
            },
        };

        let radius = match st.pet.stage {
            LifeStage::Egg => 10,
            LifeStage::Baby => 14,
            LifeStage::Child => 18,
            LifeStage::Teen => 20,
            LifeStage::Adult => 22,
            LifeStage::Elder => 24,
        };

        for y in -radius..=radius {
            for x in -radius..=radius {
                let d2 = (x * x + y * y) as f32;
                let r2 = (radius * radius) as f32;
                if d2 > r2 {
                    continue;
                }
                let dist = (d2 / r2).sqrt();
                let t = 1.0 - dist;
                let a = (mood_col.a as f32 * (0.3 + 0.7 * t)).clamp(0.0, 255.0) as u8;
                canvas.blend_over(center_x + x, center_y + y, Pixel { a, ..mood_col });
            }
        }

        let eye = Pixel {
            r: 5,
            g: 5,
            b: 8,
            a: 245,
        };
        if st.pet.flags.sleeping {
            for x in -5..=5 {
                canvas.blend_over(center_x - 6 + x, center_y - radius / 5, eye);
                canvas.blend_over(center_x + 6 + x, center_y - radius / 5, eye);
            }
            for x in -4..=4 {
                canvas.blend_over(center_x - 6 + x, center_y - radius / 5 + 1, eye);
                canvas.blend_over(center_x + 6 + x, center_y - radius / 5 + 1, eye);
            }
        } else {
            let left_x = center_x - radius / 3;
            let right_x = center_x + radius / 3;
            let eye_y = center_y - radius / 5;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    canvas.blend_over(left_x + dx, eye_y + dy, eye);
                    canvas.blend_over(right_x + dx, eye_y + dy, eye);
                }
            }
        }

        if st.pet.flags.sick {
            let p = Pixel {
                r: 140,
                g: 255,
                b: 160,
                a: 200,
            };
            for i in 0..8 {
                canvas.blend_over(center_x + radius - i, center_y - radius + i, p);
            }
        }

        if st.pet.flags.attention_call {
            let alert = Pixel {
                r: 255,
                g: 80,
                b: 80,
                a: 230,
            };
            let y = center_y - radius - 6;
            for dy in 0..6 {
                canvas.blend_over(center_x, y + dy, alert);
            }
            canvas.blend_over(center_x, y + 7, alert);
            canvas.blend_over(center_x - 1, y + 7, alert);
            canvas.blend_over(center_x + 1, y + 7, alert);
        }
    }
}

#[derive(Clone, Copy)]
struct Viewport {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

fn pet_bounce_offset_subpx(st: &GameState) -> (i32, i32) {
    let t = st.sim_ticks as f32 * 0.1;
    let x = (t.cos() * 3.0) as i32;
    let y = (t.sin() * 2.0) as i32;
    (x, y)
}

fn pet_bounce_offset_cells(st: &GameState) -> (i32, i32) {
    let (sx, sy) = pet_bounce_offset_subpx(st);
    let x = (sx as f32 / 2.0).round() as i32;
    let y = (sy as f32 / 4.0).round() as i32;
    (x, y)
}

/* -----------------------------
   UI overlay (text + meters)
------------------------------ */

fn draw_text(buf: &mut CellBuffer, x: u16, y: u16, s: &str, fg: Color, bg: Color) {
    for (i, ch) in s.chars().enumerate() {
        let xx = x.saturating_add(i as u16);
        if xx >= buf.w || y >= buf.h {
            break;
        }
        buf.set(
            xx,
            y,
            Cell {
                ch,
                fg,
                bg,
                bold: false,
            },
        );
    }
}

fn bar(value01: f32, width: usize) -> String {
    let v = value01.clamp(0.0, 1.0);
    let fill = (v * width as f32 + 0.5) as usize;
    let mut s = String::new();
    s.push('[');
    for i in 0..width {
        s.push(if i < fill { '█' } else { ' ' });
    }
    s.push(']');
    s
}

fn ui_overlay(buf: &mut CellBuffer, st: &GameState, settings: &Settings) {
    let bg = Color::Black;
    let fg = Color::White;

    let title = format!(
        "Termigotchi  |  {} ({:?})  |  Mood: {:?}",
        st.pet.name, st.pet.stage, st.pet.mood
    );
    draw_text(buf, 1, 0, &title, fg, bg);

    let m = st.pet.meters;

    let lines = [
        ("Hunger", m.hunger),
        ("Happy ", m.happiness),
        ("Health", m.health),
        ("Energy", m.energy),
        ("Hyg   ", m.hygiene),
    ];

    for (i, (name, val)) in lines.iter().enumerate() {
        let b = bar(*val / 100.0, 14);
        let s = format!("{name}: {b} {:>5.1}", val);
        draw_text(buf, 1, 2 + i as u16, &s, fg, bg);
    }

    let flags = format!(
        "Flags: sleep={} sick={} dirty={} poop={} call={}",
        st.pet.flags.sleeping as u8,
        st.pet.flags.sick as u8,
        st.pet.flags.dirty as u8,
        st.pet.flags.has_poop as u8,
        st.pet.flags.attention_call as u8
    );
    draw_text(buf, 1, 8, &flags, fg, bg);

    let help = match st.scene {
        Scene::Main => "Keys: q quit | f feed | p play | c clean | m medicine | s sleep | tab settings | h help",
        Scene::Settings => "Settings: ↑↓ select | enter apply | esc back | tab back | h help",
        Scene::Help => "Help: esc back | h close | q quit",
        Scene::Rename => "Rename: type name | enter save | esc cancel",
        Scene::Recap(_) => "Recap: any key to continue",
        Scene::Dead => "Dead: n new game | q quit",
    };
    draw_text(buf, 1, buf.h.saturating_sub(1), help, fg, bg);

    // if !settings.enable_braille {
    //     draw_text(buf, 1, 10, "Braille disabled in settings (enable_braille=false).", fg, bg);
    // }

    if matches!(st.scene, Scene::Settings) {
        draw_settings(buf, st, settings);
    }
}

/* -----------------------------
   App: load/init, loop, input, autosave
------------------------------ */

struct App {
    settings: Settings,
    rules: Rules,
    state: GameState,
    paths: Paths,
    term: Terminal,
    should_quit: bool,
    autosave_at: Instant,
}

impl App {
    fn init() -> Result<Self> {
        let paths = project_paths()?;
        let mut settings = load_settings(&paths.settings_path);
        let rules = Rules::default();

        let (mut state, loaded_last_seen) = load_or_init_save(&paths.save_path, &settings)?;

        // offline catch-up
        if let Some(last_seen) = loaded_last_seen {
            let now = Utc::now();
            let summary = catch_up(&mut state, last_seen, now, &rules);
            if summary.has_anything() {
                state.scene = Scene::Recap(summary);
            }
        }

        // ensure deterministic seed exists
        if settings.seed == 0 {
            settings.seed = 0xC0FFEE_u64;
        }

        let term = Terminal::begin()?;

        Ok(Self {
            settings,
            rules,
            state,
            paths,
            term,
            should_quit: false,
            autosave_at: Instant::now() + Duration::from_secs(10),
        })
    }

    fn run(&mut self) -> Result<()> {
        let fps = self.settings.fps_cap.max(10).min(240);
        let frame_dt = Duration::from_secs_f32(1.0 / fps as f32);
        let sim_step = Duration::from_millis(self.rules.tick_step_ms);

        let mut last_frame = Instant::now();
        let mut sim_accum = Duration::ZERO;

        while !self.should_quit {
            let _resized = self.term.resize_if_needed()?;

            // input
            let events = collect_input_nonblocking(frame_dt)?;
            for ev in events {
                if let Some(action) = map_event_to_action(&self.state.scene, ev) {
                    match action {
                        PlayerAction::Quit => {
                            self.should_quit = true;
                            break;
                        }
                        PlayerAction::SettingsToggle => {
                            if self.state.settings_cursor == 0 {
                                self.settings.enable_braille = !self.settings.enable_braille;
                            } else if self.state.settings_cursor == 1 {
                                self.state.apply(PlayerAction::RenameOpen);
                            }
                        }
                        _ => self.state.apply(action),
                    }
                } else {
                    // For recap: any key continues
                    if matches!(self.state.scene, Scene::Recap(_)) {
                        self.state.scene = Scene::Main;
                    }
                }
            }

            // sim fixed-step
            let now = Instant::now();
            let real_dt = now.saturating_duration_since(last_frame);
            last_frame = now;
            sim_accum = sim_accum.saturating_add(real_dt);

            while sim_accum >= sim_step {
                self.state.tick_fixed_step(&self.rules);
                sim_accum = sim_accum.saturating_sub(sim_step);
                if self.state.pet.flags.dead && !matches!(self.state.scene, Scene::Dead) {
                    self.state.scene = Scene::Dead;
                }
            }

            // render
            self.render_frame()?;

            // autosave
            if Instant::now() >= self.autosave_at {
                self.save_now()?;
                self.autosave_at = Instant::now() + Duration::from_secs(10);
            }

            // frame cap
            spin_sleep(frame_dt, Instant::now());
        }

        self.save_now()?;
        self.term.end()?;
        save_settings_atomic(&self.paths.settings_path, &self.settings)?;
        Ok(())
    }

    fn render_frame(&mut self) -> Result<()> {
        let bg = Color::Black;
        self.term.cur.clear(bg);

        if self.settings.enable_braille {
            self.term.canvas.clear(Pixel {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            });

            // Reserve left panel for text; pet viewport on right.
            let cols = self.term.cols as i32;
            let rows = self.term.rows as i32;

            let panel_w_cells = min(max(26, cols / 3), cols - 10);
            let pet_x_cells = panel_w_cells;
            let pet_w_cells = cols - pet_x_cells;
            let pet_h_cells = rows;

            // Pixel viewport in braille subpixels
            let vp = Viewport {
                x: pet_x_cells * 2,
                y: 0,
                w: pet_w_cells * 2,
                h: pet_h_cells * 4,
            };
            let bounce = pet_bounce_offset_subpx(&self.state);
            Renderer::draw_pet(&mut self.term.canvas, &self.state, vp, bounce);

            canvas_to_cells(
                &self.term.canvas,
                &mut self.term.cur,
                self.settings.enable_color,
                bg,
            );
        } else {
            // ASCII fallback: layered sprite approximation
            let cols = self.term.cols as i32;
            let rows = self.term.rows as i32;
            let panel_w_cells = min(max(26, cols / 3), cols - 10);
            let pet_x_cells = panel_w_cells;
            let pet_w_cells = cols - pet_x_cells;
            let pet_h_cells = rows;
            let vx = pet_x_cells + pet_w_cells / 2;
            let vy = pet_h_cells / 2;
            let bounce = pet_bounce_offset_cells(&self.state);
            draw_pet_ascii(&mut self.term.cur, &self.state, vx + bounce.0, vy + bounce.1);
        }

        // UI overlay on top
        ui_overlay(&mut self.term.cur, &self.state, &self.settings);

        // Recap overlay
        if let Scene::Recap(ref s) = self.state.scene {
            self.draw_center_box("While you were away…", &format!(
                "Simulated {} ticks\nMin hunger: {:.1}\nMin happiness: {:.1}\nMin health: {:.1}\nAttention calls: {}\nSick: {}\nDied: {}\n\nPress any key",
                s.ticks_simulated, s.hunger_min, s.happiness_min, s.health_min,
                s.attention_calls, s.became_sick, s.died
            ))?;
        }

        // Help overlay
        if let Scene::Help = self.state.scene {
            self.draw_center_box(
                "How to play",
                "Goal: keep your pet healthy and happy as it ages.\n\
    Watch the meters on the left; low stats hurt mood.\n\n\
    F Feed: +hunger/+happiness, may create poop.\n\
    P Play: +happiness, -energy, increases hunger.\n\
    C Clean: removes dirt/poop.\n\
    M Medicine: cures sickness.\n\
    S Sleep: toggle rest to regain energy.\n\
    D Discipline: clears attention calls, boosts discipline.\n\n\
    Neglect (dirty/sick/low stats) drains health over time.\n\
    Tab opens Settings (toggle render, rename pet).\n\n\
    Esc or H to close help.",
            )?;
        }

        // Rename overlay
        if let Scene::Rename = self.state.scene {
            let mut preview = self.state.name_edit.clone();
            if preview.len() < 18 {
                preview.push('_');
            }
            self.draw_center_box(
                "Rename pet",
                &format!(
                    "Type a name (max 18 chars).\n\nName: {}\n\nEnter save | Esc cancel | Backspace delete",
                    preview
                ),
            )?;
        }

        // Dead overlay
        if let Scene::Dead = self.state.scene {
            self.draw_center_box(
                "Your Termigotchi has passed on.",
                "Press N for new game, or Q to quit.",
            )?;
        }

        self.term.present(true)?;
        Ok(())
    }

    fn draw_center_box(&mut self, title: &str, body: &str) -> Result<()> {
        let w = self.term.cols;
        let h = self.term.rows;

        let bw = min(60, w.saturating_sub(4));
        let bh = min(18, h.saturating_sub(4));

        let x0 = (w - bw) / 2;
        let y0 = (h - bh) / 2;

        // border
        for x in x0..x0 + bw {
            self.term.cur.set(
                x,
                y0,
                Cell {
                    ch: '─',
                    fg: Color::White,
                    bg: Color::Black,
                    bold: false,
                },
            );
            self.term.cur.set(
                x,
                y0 + bh - 1,
                Cell {
                    ch: '─',
                    fg: Color::White,
                    bg: Color::Black,
                    bold: false,
                },
            );
        }
        for y in y0..y0 + bh {
            self.term.cur.set(
                x0,
                y,
                Cell {
                    ch: '│',
                    fg: Color::White,
                    bg: Color::Black,
                    bold: false,
                },
            );
            self.term.cur.set(
                x0 + bw - 1,
                y,
                Cell {
                    ch: '│',
                    fg: Color::White,
                    bg: Color::Black,
                    bold: false,
                },
            );
        }
        self.term.cur.set(
            x0,
            y0,
            Cell {
                ch: '┌',
                fg: Color::White,
                bg: Color::Black,
                bold: false,
            },
        );
        self.term.cur.set(
            x0 + bw - 1,
            y0,
            Cell {
                ch: '┐',
                fg: Color::White,
                bg: Color::Black,
                bold: false,
            },
        );
        self.term.cur.set(
            x0,
            y0 + bh - 1,
            Cell {
                ch: '└',
                fg: Color::White,
                bg: Color::Black,
                bold: false,
            },
        );
        self.term.cur.set(
            x0 + bw - 1,
            y0 + bh - 1,
            Cell {
                ch: '┘',
                fg: Color::White,
                bg: Color::Black,
                bold: false,
            },
        );

        // title
        draw_text(
            &mut self.term.cur,
            x0 + 2,
            y0 + 1,
            title,
            Color::White,
            Color::Black,
        );

        // body
        let mut yy = y0 + 3;
        for line in body.lines() {
            if yy >= y0 + bh - 1 {
                break;
            }
            draw_text(
                &mut self.term.cur,
                x0 + 2,
                yy,
                line,
                Color::White,
                Color::Black,
            );
            yy += 1;
        }

        Ok(())
    }

    fn save_now(&self) -> Result<()> {
        let now = Utc::now();
        let save = SaveFile {
            version: SAVE_VERSION,
            last_seen_utc: now,
            state: self.state.clone(),
        };
        save_atomic(&self.paths.save_path, &save)?;
        Ok(())
    }
}

fn load_or_init_save(
    path: &Path,
    settings: &Settings,
) -> Result<(GameState, Option<DateTime<Utc>>)> {
    if let Ok(s) = fs::read_to_string(path) {
        if let Ok(save) = serde_json::from_str::<SaveFile>(&s) {
            return Ok((save.state, Some(save.last_seen_utc)));
        }
    }
    Ok((GameState::new(settings.seed), None))
}

fn save_atomic(path: &Path, save: &SaveFile) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(save)?;
    fs::write(&tmp, data)?;
    atomic_rename(&tmp, path)?;
    Ok(())
}

/* -----------------------------
   Input handling
------------------------------ */

#[derive(Clone, Debug)]
struct InputEvent {
    key: KeyCode,
    mods: KeyModifiers,
}

fn collect_input_nonblocking(max_frame_time: Duration) -> Result<Vec<InputEvent>> {
    let mut out = Vec::new();

    // poll with a tiny timeout so we stay responsive
    let timeout = min(Duration::from_millis(1), max_frame_time);
    while event::poll(timeout)? {
        match event::read()? {
            Event::Key(k) => {
                if k.kind == KeyEventKind::Press || k.kind == KeyEventKind::Repeat {
                    out.push(InputEvent {
                        key: k.code,
                        mods: k.modifiers,
                    });
                    if out.len() >= 32 {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn map_event_to_action(scene: &Scene, ev: InputEvent) -> Option<PlayerAction> {
    if matches!(scene, Scene::Rename) {
        return match ev.key {
            KeyCode::Enter => Some(PlayerAction::RenameCommit),
            KeyCode::Esc => Some(PlayerAction::RenameCancel),
            KeyCode::Backspace => Some(PlayerAction::RenameBackspace),
            KeyCode::Char(ch) => {
                if (ch.is_ascii() && !ch.is_ascii_control()) || ch == ' ' {
                    Some(PlayerAction::RenameChar(ch))
                } else {
                    None
                }
            }
            _ => None,
        };
    }

    // Global
    if matches!(ev.key, KeyCode::Char('p') | KeyCode::Char('P'))
        && ev.mods.contains(KeyModifiers::CONTROL)
    {
        return Some(PlayerAction::DebugKill);
    }
    match ev.key {
        KeyCode::Char('h') | KeyCode::Char('H') => return Some(PlayerAction::HelpToggle),
        KeyCode::Char('q') | KeyCode::Char('Q') => return Some(PlayerAction::Quit),
        KeyCode::Esc => return Some(PlayerAction::Back),
        _ => {}
    }

    match scene {
        Scene::Main => match ev.key {
            KeyCode::Char('f') | KeyCode::Char('F') => Some(PlayerAction::Feed("kibble")),
            KeyCode::Char('p') | KeyCode::Char('P') => Some(PlayerAction::PlayAny),
            KeyCode::Char('c') | KeyCode::Char('C') => Some(PlayerAction::Clean),
            KeyCode::Char('m') | KeyCode::Char('M') => Some(PlayerAction::Medicine),
            KeyCode::Char('s') | KeyCode::Char('S') => Some(PlayerAction::SleepToggle),
            KeyCode::Char('d') | KeyCode::Char('D') => Some(PlayerAction::Discipline),
            KeyCode::Tab => Some(PlayerAction::SettingsOpen),
            _ => None,
        },
        Scene::Settings => match ev.key {
            KeyCode::Up => Some(PlayerAction::SettingsMove(-1)),
            KeyCode::Down => Some(PlayerAction::SettingsMove(1)),
            KeyCode::Enter => Some(PlayerAction::SettingsToggle),
            KeyCode::Esc => Some(PlayerAction::Back),
            KeyCode::Tab => Some(PlayerAction::Back),
            _ => None,
        },
        Scene::Help => match ev.key {
            KeyCode::Esc => Some(PlayerAction::Back),
            _ => None,
        },
        Scene::Dead => match ev.key {
            KeyCode::Char('n') | KeyCode::Char('N') => Some(PlayerAction::NewGame),
            _ => None,
        },
        Scene::Rename => None,
        Scene::Recap(_) => None,
    }
}

/* -----------------------------
   Shop catalog + UI
------------------------------ */

fn draw_settings(buf: &mut CellBuffer, st: &GameState, settings: &Settings) {
    let bg = Color::Black;
    let fg = Color::White;
    let hi = Color::Yellow;

    let start_x = 1;
    let start_y = 11;

    draw_text(buf, start_x, start_y, "Settings", fg, bg);

    let selected = st.settings_cursor;
    let render_mode = if settings.enable_braille {
        "Braille"
    } else {
        "ASCII"
    };
    let line = format!(
        "{} Render: {}",
        if selected == 0 { ">" } else { " " },
        render_mode
    );
    draw_text(
        buf,
        start_x,
        start_y + 2,
        &line,
        if selected == 0 { hi } else { fg },
        bg,
    );

    let mut name_display = st.pet.name.clone();
    if name_display.len() > 16 {
        name_display.truncate(15);
        name_display.push_str("...");
    }
    let name_line = format!(
        "{} Name: {}",
        if selected == 1 { ">" } else { " " },
        name_display
    );
    draw_text(
        buf,
        start_x,
        start_y + 3,
        &name_line,
        if selected == 1 { hi } else { fg },
        bg,
    );
}

fn draw_pet_ascii(buf: &mut CellBuffer, st: &GameState, cx: i32, cy: i32) {
    let bg = Color::Black;
    let fg = Color::White;

    let (w, h) = (15i32, 9i32);
    let x0 = cx - w / 2;
    let y0 = cy - h / 2;

    let mut grid = [
        "       _____     ",
        "     /       \\   ",
        "    /  o   o  \\  ",
        "   |     ^     | ",
        "   |   \\___/   | ",
        "    \\         /  ",
        "     \\_______/   ",
        "                 ",
        "                 ",
    ];

    // mood tweak: mouth shape
    if matches!(st.pet.mood, Mood::Sad | Mood::Sick | Mood::Angry) {
        grid[4] = "   |   /___\\   | ";
    } else if matches!(st.pet.mood, Mood::Happy) {
        grid[4] = "   |   \\___/   | ";
    }

    for (yy, line) in grid.iter().enumerate() {
        let y = y0 + yy as i32;
        if y < 0 || y >= buf.h as i32 {
            continue;
        }
        let mut x = x0;
        for ch in line.chars() {
            if x >= 0 && x < buf.w as i32 {
                buf.set(
                    x as u16,
                    y as u16,
                    Cell {
                        ch,
                        fg,
                        bg,
                        bold: false,
                    },
                );
            }
            x += 1;
        }
    }

    if st.pet.flags.attention_call {
        let ax = cx;
        let ay = y0 - 2;
        if ay >= 0 && ay < buf.h as i32 && ax >= 0 && ax < buf.w as i32 {
            buf.set(
                ax as u16,
                ay as u16,
                Cell {
                    ch: '!',
                    fg: Color::Red,
                    bg,
                    bold: true,
                },
            );
        }
    }
}

/* -----------------------------
   Frame pacing helper
------------------------------ */

fn spin_sleep(target: Duration, now: Instant) {
    let end = now + target;
    loop {
        let t = Instant::now();
        if t >= end {
            break;
        }
        let left = end - t;
        if left > Duration::from_millis(2) {
            std::thread::sleep(Duration::from_millis(1));
        } else {
            std::hint::spin_loop();
        }
    }
}
