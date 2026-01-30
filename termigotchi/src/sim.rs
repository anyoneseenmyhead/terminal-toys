use crate::model::{
    CatchupSummary, DecayRates, Flags, GameState, LifeStage, Meters, Mood, Rules, Scene,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};

#[derive(Clone, Debug)]
pub(crate) enum PlayerAction {
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
    pub(crate) fn apply(&mut self, action: PlayerAction) {
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
                self.pet.meters.discipline =
                    (self.pet.meters.discipline + 12.0).clamp(0.0, 100.0);
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

    pub(crate) fn tick_fixed_step(&mut self, rules: &Rules) {
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

pub(crate) fn catch_up(
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
