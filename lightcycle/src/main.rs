mod ai;
mod input;
mod render;
mod sim;
mod util;

use std::io::{Stdout, stdout};
use std::panic;
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::cursor::{Hide, Show};
use crossterm::event;
use crossterm::execute;
use crossterm::terminal::{
    self, Clear, ClearType, DisableLineWrap, EnableLineWrap, EnterAlternateScreen,
    LeaveAlternateScreen,
};

use input::{InputAction, InputState, poll_input};
use render::{HudData, RenderConfig, Renderer};
use sim::{Round, RoundOutcome, SimConfig};
use util::{default_seed, splitmix64};

#[derive(Parser, Debug)]
#[command(
    name = "lightcycle",
    version,
    about = "TRON-style terminal lightcycle arena",
    long_about = "TRON-style terminal lightcycle arena.\n\nIn game mode, you control cycle C1. The round waits for your first direction input before simulation starts.",
    after_help = "Controls:\n  WASD / Arrow keys   Steer (90-degree turns)\n  Space               Pause/resume\n  R                   Restart round\n  V                   Toggle wall visibility (debug)\n  E                   Drop wall segment (limited charges)\n  Q / Ctrl+C          Quit\n\nExamples:\n  lightcycle\n  lightcycle --screensaver --cycles 10\n  lightcycle --speed 16 --fps 120 --seed 42\n  lightcycle --no-color --glow"
)]
struct Cli {
    #[arg(long, help = "Run in AI-only screensaver mode")]
    screensaver: bool,
    #[arg(
        long,
        default_value_t = 6,
        help = "Total number of cycles in the arena (includes player in game mode)"
    )]
    cycles: usize,
    #[arg(
        long,
        default_value_t = 60,
        help = "Render frame cap in frames per second"
    )]
    fps: u32,
    #[arg(
        long,
        default_value_t = 12,
        help = "Simulation movement speed in moves per second"
    )]
    speed: u32,
    #[arg(long, help = "Base RNG seed for deterministic rounds")]
    seed: Option<u64>,
    #[arg(
        long,
        default_value_t = 1,
        help = "Inner margin around the playfield (cells)"
    )]
    margin: u16,
    #[arg(long, help = "Disable ANSI color output")]
    no_color: bool,
    #[arg(long, help = "Enable lightweight glow accents around cycle heads")]
    glow: bool,
    #[arg(
        long,
        help = "Enable stricter simultaneous collision rules (head swaps collide)"
    )]
    strict_sim: bool,
    #[arg(
        long,
        default_value_t = true,
        help = "Disallow immediate 180-degree turns except when no safe non-reverse move exists"
    )]
    no_reverse: bool,
}

struct TerminalGuard;

impl TerminalGuard {
    fn setup(stdout: &mut Stdout) -> std::io::Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(
            stdout,
            EnterAlternateScreen,
            Hide,
            DisableLineWrap,
            Clear(ClearType::All)
        )?;
        install_panic_hook();
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = restore_terminal();
    }
}

fn restore_terminal() -> std::io::Result<()> {
    let mut out = stdout();
    execute!(out, Show, EnableLineWrap, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    Ok(())
}

fn install_panic_hook() {
    let default = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        default(info);
    }));
}

struct AppState {
    cli: Cli,
    sim_cfg: SimConfig,
    base_seed: u64,
    round_counter: u64,
    round: Option<Round>,
    renderer: Renderer,
    input: InputState,
    paused: bool,
    awaiting_first_direction: bool,
    show_walls: bool,
    wins: u64,
    losses: u64,
    banner: Option<RoundOutcome>,
    banner_start: Option<Instant>,
    fps_estimate: f32,
    last_term_size: (u16, u16),
}

impl AppState {
    fn new(cli: Cli, term_w: u16, term_h: u16) -> Self {
        let base_seed = cli.seed.unwrap_or_else(default_seed);

        let screensaver = cli.screensaver;
        let sim_cfg = SimConfig {
            cycles: cli.cycles.max(2),
            margin: cli.margin,
            no_reverse: cli.no_reverse,
            strict_sim: cli.strict_sim,
            allow_reverse_if_only_safe: true,
            screensaver,
        };

        let mut app = Self {
            cli,
            sim_cfg,
            base_seed,
            round_counter: 0,
            round: None,
            renderer: Renderer::new(term_w, term_h),
            input: InputState::default(),
            paused: false,
            awaiting_first_direction: !screensaver,
            show_walls: true,
            wins: 0,
            losses: 0,
            banner: None,
            banner_start: None,
            fps_estimate: 0.0,
            last_term_size: (term_w, term_h),
        };
        app.restart_round(term_w, term_h);
        app
    }

    fn restart_round(&mut self, term_w: u16, term_h: u16) {
        let round_seed = splitmix64(self.base_seed ^ self.round_counter);
        self.round = Round::new(&self.sim_cfg, term_w, term_h, round_seed);
        self.round_counter = self.round_counter.wrapping_add(1);
        self.banner = None;
        self.banner_start = None;
        self.awaiting_first_direction = !self.sim_cfg.screensaver;
    }

    fn handle_resize(&mut self) -> std::io::Result<()> {
        let (w, h) = terminal::size()?;
        if (w, h) != self.last_term_size {
            self.last_term_size = (w, h);
            self.renderer.resize(w, h);
            self.restart_round(w, h);
        }
        Ok(())
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("lightcycle error: {e}");
        std::process::exit(1);
    }
}

fn run() -> std::io::Result<()> {
    let cli = Cli::parse();
    let seed = cli.seed.unwrap_or_else(default_seed);
    eprintln!("lightcycle seed={seed}");

    let mut out = stdout();
    let _guard = TerminalGuard::setup(&mut out)?;

    let (w, h) = terminal::size()?;
    let mut app = AppState::new(cli, w, h);

    let target_fps = app.cli.fps.max(1);
    let sim_speed = app.cli.speed.max(1);
    let frame_dt = Duration::from_secs_f64(1.0 / target_fps as f64);
    let tick_dt = Duration::from_secs_f64(1.0 / sim_speed as f64);

    let mut quit = false;
    let mut last = Instant::now();
    let mut last_render = Instant::now();
    let mut accumulator = Duration::ZERO;

    while !quit {
        app.input.clear_frame_flags();
        poll_input(&mut app.input)?;
        if app.input.resized {
            app.handle_resize()?;
        }

        let actions = app.input.actions.clone();
        for action in actions {
            match action {
                InputAction::Quit => quit = true,
                InputAction::Restart => {
                    app.restart_round(app.last_term_size.0, app.last_term_size.1)
                }
                InputAction::PauseToggle => app.paused = !app.paused,
                InputAction::ToggleWalls => app.show_walls = !app.show_walls,
                InputAction::DropWall => {
                    if let Some(round) = app.round.as_mut() {
                        round.queue_drop_wall();
                    }
                }
            }
        }
        if quit {
            break;
        }

        let now = Instant::now();
        let dt = now.saturating_duration_since(last);
        last = now;
        accumulator += dt;

        if !app.paused {
            if app.awaiting_first_direction {
                if let Some(dir) = app.input.take_buffered_dir() {
                    if let Some(round) = app.round.as_mut() {
                        round.set_player_start_dir(dir);
                    }
                    app.awaiting_first_direction = false;
                }
                accumulator = Duration::ZERO;
            }
            while accumulator >= tick_dt {
                accumulator -= tick_dt;
                if let Some(round) = app.round.as_mut() {
                    if !round.screensaver {
                        let dir = app.input.take_buffered_dir();
                        round.queue_player_dir(dir);
                    }
                    let tick = round.tick();
                    if let Some(outcome) = tick.outcome
                        && app.banner.is_none()
                    {
                        app.banner = Some(outcome);
                        app.banner_start = Some(Instant::now());
                        match outcome {
                            RoundOutcome::Victory => app.wins = app.wins.saturating_add(1),
                            RoundOutcome::Defeat => app.losses = app.losses.saturating_add(1),
                        }
                    }
                }
            }
        }

        if let (Some(_banner), Some(started)) = (app.banner, app.banner_start)
            && !app.paused
            && started.elapsed() >= Duration::from_millis(1000)
        {
            app.restart_round(app.last_term_size.0, app.last_term_size.1);
        }

        if now.duration_since(last_render) >= frame_dt {
            let elapsed = now.duration_since(last_render).as_secs_f32();
            last_render = now;
            if elapsed > 0.0 {
                let inst_fps = 1.0 / elapsed;
                if app.fps_estimate <= 0.1 {
                    app.fps_estimate = inst_fps;
                } else {
                    app.fps_estimate = app.fps_estimate * 0.9 + inst_fps * 0.1;
                }
            }

            app.handle_resize()?;
            let alive = app.round.as_ref().map_or(0, |r| r.alive_count());
            let controlled_cycle = app
                .round
                .as_ref()
                .and_then(|r| r.cycles.iter().find(|c| c.is_player).map(|c| c.id));
            let hud = HudData {
                fps: app.fps_estimate,
                speed: sim_speed,
                alive,
                wins: app.wins,
                losses: app.losses,
                seed: app.base_seed,
                controlled_cycle,
                paused: app.paused,
                waiting_for_start: app.awaiting_first_direction,
            };
            let rcfg = RenderConfig {
                no_color: app.cli.no_color,
                glow: app.cli.glow,
            };
            app.renderer.draw(
                &mut out,
                app.round.as_ref(),
                &hud,
                app.show_walls,
                rcfg,
                app.banner,
            )?;
        }

        let until_render = frame_dt.saturating_sub(now.saturating_duration_since(last_render));
        let until_tick = if app.paused {
            frame_dt
        } else {
            tick_dt.saturating_sub(accumulator)
        };
        let sleep_for = until_render.min(until_tick).min(Duration::from_millis(8));
        if sleep_for > Duration::from_millis(0) {
            thread::sleep(sleep_for);
        }
    }

    if event::poll(Duration::from_millis(0))? {
        let _ = event::read();
    }

    Ok(())
}
