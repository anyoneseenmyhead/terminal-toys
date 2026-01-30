use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) const SAVE_VERSION: u32 = 1;
pub(crate) const GAME_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum Scene {
    Main,
    Settings,
    Help,
    Rename,
    Recap(CatchupSummary),
    Dead,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum LifeStage {
    Egg,
    Baby,
    Child,
    Teen,
    Adult,
    Elder,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(crate) enum Mood {
    Happy,
    Okay,
    Sad,
    Angry,
    Sick,
    Sleepy,
    Bored,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(crate) struct Meters {
    pub(crate) hunger: f32,
    pub(crate) happiness: f32,
    pub(crate) health: f32,
    pub(crate) energy: f32,
    pub(crate) hygiene: f32,
    pub(crate) discipline: f32,
    pub(crate) bond: f32,
    pub(crate) weight: f32,
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
pub(crate) struct Flags {
    pub(crate) sleeping: bool,
    pub(crate) sick: bool,
    pub(crate) dirty: bool,
    pub(crate) attention_call: bool,
    pub(crate) has_poop: bool,
    pub(crate) dead: bool,
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
pub(crate) struct History {
    pub(crate) neglected_events: u32,
    pub(crate) play_events: u32,
    pub(crate) feed_events: u32,
    pub(crate) clean_events: u32,
    pub(crate) discipline_events: u32,
    pub(crate) sickness_events: u32,
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
pub(crate) struct Pet {
    pub(crate) name: String,
    pub(crate) species_id: String,
    pub(crate) stage: LifeStage,
    pub(crate) age_ticks: u64,
    pub(crate) mood: Mood,
    pub(crate) meters: Meters,
    pub(crate) flags: Flags,
    pub(crate) history: History,
}

impl Pet {
    pub(crate) fn new_default() -> Self {
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
pub(crate) struct RNGState {
    pub(crate) seed: u64,
    pub(crate) event_counter: u64,
}

impl RNGState {
    pub(crate) fn new(seed: u64) -> Self {
        Self {
            seed,
            event_counter: 0,
        }
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
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

    pub(crate) fn next_f32(&mut self) -> f32 {
        // [0,1)
        let v = self.next_u64() >> 40; // 24 bits
        (v as f32) / ((1u64 << 24) as f32)
    }

    pub(crate) fn roll(&mut self, p: f32) -> bool {
        self.next_f32() < p.clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(crate) struct DecayRates {
    pub(crate) hunger: f32,
    pub(crate) happiness: f32,
    pub(crate) energy: f32,
    pub(crate) hygiene: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Rules {
    pub(crate) tick_step_ms: u64,     // 250ms typical
    pub(crate) catchup_step_ms: u64,  // 5000ms typical
    pub(crate) catchup_max_secs: i64, // 7 days typical
    pub(crate) meter_rate_scale: f32, // 1.0 = normal, lower = slower stat changes
    pub(crate) decay_by_stage: BTreeMap<LifeStage, DecayRates>,
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
pub(crate) struct GameState {
    pub(crate) version: u32,
    pub(crate) pet: Pet,
    pub(crate) scene: Scene,
    pub(crate) rng: RNGState,
    pub(crate) sim_ticks: u64,
    pub(crate) last_action_at_tick: u64,
    #[serde(default)]
    pub(crate) settings_cursor: usize,
    #[serde(default)]
    pub(crate) name_edit: String,
}

impl GameState {
    pub(crate) fn new(seed: u64) -> Self {
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
pub(crate) struct SaveFile {
    pub(crate) version: u32,
    pub(crate) last_seen_utc: DateTime<Utc>,
    pub(crate) state: GameState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CatchupSummary {
    pub(crate) ticks_simulated: u64,
    pub(crate) became_sick: bool,
    pub(crate) attention_calls: u32,
    pub(crate) died: bool,
    pub(crate) hunger_min: f32,
    pub(crate) happiness_min: f32,
    pub(crate) health_min: f32,
}

impl CatchupSummary {
    pub(crate) fn new() -> Self {
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

    pub(crate) fn has_anything(&self) -> bool {
        self.ticks_simulated > 0
            && (self.became_sick
                || self.attention_calls > 0
                || self.died
                || self.hunger_min < 40.0
                || self.happiness_min < 40.0
                || self.health_min < 60.0)
    }

    pub(crate) fn record(&mut self, st: &GameState) {
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
