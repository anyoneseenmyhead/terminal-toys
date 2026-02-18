// src/main.rs
use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    queue,
    style::{Attribute, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, DisableLineWrap, EnableLineWrap, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::{
    cmp::{max, min},
    env,
    io::{self, Write},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug)]
struct Cell {
    ch: char,
    fg: crossterm::style::Color,
    attr: Attribute,
}
impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: crossterm::style::Color::White,
            attr: Attribute::Reset,
        }
    }
}

#[derive(Clone)]
struct Frame {
    w: u16,
    h: u16,
    cells: Vec<Cell>,
}
impl Frame {
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
    fn set(&mut self, x: u16, y: u16, ch: char, fg: crossterm::style::Color, attr: Attribute) {
        if x >= self.w || y >= self.h {
            return;
        }
        let i = self.idx(x, y);
        self.cells[i] = Cell { ch, fg, attr };
    }
    fn write_str(
        &mut self,
        mut x: u16,
        y: u16,
        s: &str,
        fg: crossterm::style::Color,
        attr: Attribute,
    ) {
        for ch in s.chars() {
            if x >= self.w {
                break;
            }
            self.set(x, y, ch, fg, attr);
            x += 1;
        }
    }
    fn clear(&mut self, ch: char, fg: crossterm::style::Color, attr: Attribute) {
        for c in &mut self.cells {
            c.ch = ch;
            c.fg = fg;
            c.attr = attr;
        }
    }
}

#[derive(Clone, Copy)]
struct Reel {
    // phase moves continuously; displayed symbol depends on phase
    phase: f32,
    speed: f32,
    // stopping behavior
    stopping: bool,
    stop_at_phase: f32,
    stop_t: f32,      // time remaining to stop (seconds)
    stop_total: f32,  // total stop duration
    final_index: usize,
}

#[derive(Clone, Copy)]
struct WinInfo {
    payout: i64,
    line: [usize; 3],
    kind: WinKind,
}
#[derive(Clone, Copy)]
enum WinKind {
    None,
    ThreeMatch,
    TwoMatch,
}

const REELS: usize = 3;

// Symbols are intentionally terminal-friendly.
// Weighting: more common symbols earlier.
const SYMBOLS: &[char] = &['7', '♦', '♥', '♣', '♠', '★', '✿', '☘', '◈', '○'];
const SYMBOL_WEIGHTS: &[u32] = &[1, 3, 3, 4, 4, 5, 6, 6, 8, 10];

// Smear ramp used at high speed.
const SMEAR: &[char] = &['░', '▒', '▓', '█'];

fn main() -> Result<()> {
    let seed = parse_seed();
    let mut rng = ChaCha8Rng::seed_from_u64(seed);

    let mut stdout = io::stdout();
    terminal::enable_raw_mode()?;
    crossterm::execute!(stdout, EnterAlternateScreen, cursor::Hide, DisableLineWrap)?;

    let mut last_size = terminal::size()?;
    let mut back = Frame::new(last_size.0, last_size.1);
    let mut front = Frame::new(last_size.0, last_size.1);

    let mut credits: i64 = 1000;
    let mut bet: i64 = 25;
    let mut spinning = false;
    let mut reels = init_reels(&mut rng);
    let mut last_win = WinInfo {
        payout: 0,
        line: [0, 0, 0],
        kind: WinKind::None,
    };

    let mut last = Instant::now();
    let app_start = Instant::now();
    let mut spin_cooldown = 0.0f32;

    'app: loop {
        // resize handling
        let sz = terminal::size()?;
        if sz != last_size {
            last_size = sz;
            back = Frame::new(sz.0, sz.1);
            front = Frame::new(sz.0, sz.1);
            // force full redraw by clearing front to something different
            front.clear('\0', crossterm::style::Color::White, Attribute::Reset);
        }

        // input
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => break 'app,
                    KeyCode::Char(' ') | KeyCode::Enter => {
                        if !spinning && spin_cooldown <= 0.0 && credits >= bet && bet > 0 {
                            credits -= bet;
                            start_spin(&mut reels, &mut rng);
                            spinning = true;
                            last_win = WinInfo {
                                payout: 0,
                                line: [0, 0, 0],
                                kind: WinKind::None,
                            };
                        }
                    }
                    KeyCode::Up => {
                        bet = min(credits.max(1), bet + 5);
                    }
                    KeyCode::Down => {
                        bet = max(1, bet - 5);
                    }
                    KeyCode::Char('r') => {
                        credits = 1000;
                        bet = 25;
                        spinning = false;
                        reels = init_reels(&mut rng);
                        last_win = WinInfo {
                            payout: 0,
                            line: [0, 0, 0],
                            kind: WinKind::None,
                        };
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        // dt
        let now = Instant::now();
        let mut dt = (now - last).as_secs_f32();
        last = now;
        dt = dt.clamp(0.0, 0.05); // keep stable over ssh hiccups

        if spin_cooldown > 0.0 {
            spin_cooldown -= dt;
        }

        // update
        if spinning {
            let done = update_spin(&mut reels, dt);
            if done {
                spinning = false;
                // compute win on middle row
                let line = [
                    symbol_at(&reels[0], 1),
                    symbol_at(&reels[1], 1),
                    symbol_at(&reels[2], 1),
                ];
                last_win = evaluate_line(line, bet);
                credits += last_win.payout;
                spin_cooldown = 0.25;
            }
        }

        // render
        back.clear(' ', crossterm::style::Color::White, Attribute::Reset);
        render_ui(
            &mut back,
            last_size,
            credits,
            bet,
            spinning,
            &reels,
            last_win,
            seed,
            app_start.elapsed().as_secs_f32(),
        );
        flush_diff(&mut stdout, &back, &mut front)?;

        // frame cap
        std::thread::sleep(Duration::from_millis(16));
    }

    // restore
    crossterm::execute!(
        stdout,
        EndSynchronizedUpdate,
        EnableLineWrap,
        cursor::Show,
        LeaveAlternateScreen
    )?;
    terminal::disable_raw_mode()?;
    Ok(())
}

fn parse_seed() -> u64 {
    // --seed <u64>
    let mut args = env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--seed" {
            if let Some(v) = args.next() {
                if let Ok(n) = v.parse::<u64>() {
                    return n;
                }
            }
        }
    }
    // time-based
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0xC0FFEE)
}

fn init_reels(rng: &mut ChaCha8Rng) -> [Reel; REELS] {
    let mut reels = [Reel {
        phase: 0.0,
        speed: 0.0,
        stopping: false,
        stop_at_phase: 0.0,
        stop_t: 0.0,
        stop_total: 0.0,
        final_index: 0,
    }; REELS];
    for (i, r) in reels.iter_mut().enumerate() {
        r.phase = rng.gen_range(0.0..(SYMBOLS.len() as f32));
        r.speed = 0.0;
        r.final_index = i % SYMBOLS.len();
    }
    reels
}

fn start_spin(reels: &mut [Reel; REELS], rng: &mut ChaCha8Rng) {
    // Decide final middle-row symbol for each reel first, then animate into alignment.
    for (i, r) in reels.iter_mut().enumerate() {
        r.stopping = false;
        r.stop_t = 0.0;
        r.stop_total = 0.0;

        r.speed = rng.gen_range(20.0..34.0) + (i as f32) * 2.5;
        r.final_index = weighted_pick(rng);

        // we want middle row (offset +1) to land on final_index
        // phase maps to index floor(phase) => top row
        // middle row index is floor(phase)+1
        // so we need floor(phase)+1 == final_index mod N, with phase aligned on integer boundary.
        let n = SYMBOLS.len() as i32;
        let want_top = (r.final_index as i32 - 1).rem_euclid(n) as usize;

        // choose a future integer phase that satisfies the top index at stop
        let current_top = (r.phase.floor() as i32).rem_euclid(n) as usize;
        let mut delta = (want_top as i32 - current_top as i32).rem_euclid(n);
        if delta < 0 {
            delta += n;
        }
        // add extra full rotations for drama
        let extra = rng.gen_range(3..7) * n;
        let target_top_steps = (delta as i32 + extra) as f32;

        // stop_at_phase is an integer boundary (top row aligns)
        let target_phase = r.phase.floor() + target_top_steps;
        r.stop_at_phase = target_phase;
    }

    // Stagger stop times: each reel begins decel later and stops later.
    for (i, r) in reels.iter_mut().enumerate() {
        let delay = 0.35 + (i as f32) * 0.35;
        let decel = 0.85 + (i as f32) * 0.12;
        r.stopping = true;
        r.stop_total = decel;
        r.stop_t = delay + decel;
    }
}

fn update_spin(reels: &mut [Reel; REELS], dt: f32) -> bool {
    let mut all_stopped = true;

    for r in reels.iter_mut() {
        if !r.stopping {
            continue;
        }

        if r.stop_t > 0.0 {
            r.stop_t -= dt;
        }

        // Before decel window, keep spinning fast.
        let decel_window = r.stop_total;
        let in_decel = r.stop_t <= decel_window;

        if !in_decel {
            r.phase += r.speed * dt;
            all_stopped = false;
            continue;
        }

        // Ease-out deceleration to land exactly on stop_at_phase.
        // We compute how far remaining (in phase units) and choose a speed that approaches 0.
        let t = (r.stop_t / decel_window).clamp(0.0, 1.0); // 1 -> start of decel, 0 -> end
        let ease = t * t * (3.0 - 2.0 * t); // smoothstep

        let remaining = (r.stop_at_phase - r.phase).max(0.0);
        let min_step = 0.5; // avoid stalling
        let desired_step = remaining.max(min_step);

        // Scale speed down by easing, but ensure we reach target by the end.
        let target_speed = (desired_step / dt).min(60.0) * ease;

        r.speed = r.speed.min(60.0).max(target_speed);
        r.phase += r.speed * dt;

        if r.stop_t <= 0.0 {
            // Snap cleanly to final boundary.
            r.phase = r.stop_at_phase;
            r.speed = 0.0;
        } else {
            all_stopped = false;
        }
    }

    all_stopped
}

fn weighted_pick(rng: &mut ChaCha8Rng) -> usize {
    let total: u32 = SYMBOL_WEIGHTS.iter().sum();
    let mut roll = rng.gen_range(0..total);
    for (i, w) in SYMBOL_WEIGHTS.iter().enumerate() {
        if roll < *w {
            return i;
        }
        roll -= *w;
    }
    SYMBOLS.len() - 1
}

fn symbol_at(reel: &Reel, row: usize) -> usize {
    // row: 0 top, 1 mid, 2 bot
    let n = SYMBOLS.len() as i32;
    let top = reel.phase.floor() as i32;
    let idx = (top + row as i32).rem_euclid(n) as usize;
    idx
}

fn evaluate_line(line: [usize; 3], bet: i64) -> WinInfo {
    let a = line[0];
    let b = line[1];
    let c = line[2];

    if a == b && b == c {
        // rarer symbols pay more: inverse of weight-ish
        let w = SYMBOL_WEIGHTS[a] as i64;
        let mult = (18 - min(16, w as i64)).max(2);
        let payout = bet * mult;
        return WinInfo {
            payout,
            line: [a, b, c],
            kind: WinKind::ThreeMatch,
        };
    }

    if a == b || b == c || a == c {
        let payout = (bet * 2).max(1);
        return WinInfo {
            payout,
            line: [a, b, c],
            kind: WinKind::TwoMatch,
        };
    }

    WinInfo {
        payout: 0,
        line: [a, b, c],
        kind: WinKind::None,
    }
}

fn render_ui(
    f: &mut Frame,
    size: (u16, u16),
    credits: i64,
    bet: i64,
    spinning: bool,
    reels: &[Reel; REELS],
    last_win: WinInfo,
    seed: u64,
    time_s: f32,
) {
    let (w, h) = size;
    if w < 64 || h < 22 {
        f.write_str(
            2,
            1,
            "Resize terminal (need ~64x22). Press q to quit.",
            crossterm::style::Color::Yellow,
            Attribute::Bold,
        );
        return;
    }

    // Layout
    let top = 1u16;
    let left = (w / 2).saturating_sub(30);
    let box_w = 60u16;
    let box_h = 17u16;

    // Ambient background gradient.
    for yy in 0..h {
        let shade = if yy < h / 3 {
            '.'
        } else if yy < (h * 2) / 3 {
            '·'
        } else {
            ' '
        };
        let color = if yy < h / 2 {
            crossterm::style::Color::DarkBlue
        } else {
            crossterm::style::Color::Black
        };
        for xx in 0..w {
            if (xx + yy) % 11 == 0 {
                f.set(xx, yy, shade, color, Attribute::Reset);
            }
        }
    }

    // Title
    f.write_str(
        left + 3,
        top,
        "NEON SLOTS 9000",
        crossterm::style::Color::Cyan,
        Attribute::Bold,
    );
    f.write_str(
        left + 21,
        top,
        "  Space/Enter: spin   Up/Down: bet   r: reset   q: quit",
        crossterm::style::Color::DarkGrey,
        Attribute::Reset,
    );

    // Cabinet body + shadow for depth.
    for yy in (top + 2)..=(top + box_h) {
        f.set(
            left + box_w,
            yy,
            '▓',
            crossterm::style::Color::DarkGrey,
            Attribute::Reset,
        );
    }
    for xx in (left + 1)..=left + box_w {
        f.set(
            xx,
            top + box_h + 1,
            '▓',
            crossterm::style::Color::DarkGrey,
            Attribute::Reset,
        );
    }
    draw_box(
        f,
        left,
        top + 1,
        box_w,
        box_h,
        crossterm::style::Color::Yellow,
    );
    draw_box(
        f,
        left + 1,
        top + 2,
        box_w - 2,
        box_h - 2,
        crossterm::style::Color::DarkRed,
    );

    // Reel window
    let window_x = left + 4;
    let window_y = top + 4;
    let window_w = box_w - 8;
    let window_h = 9u16;

    draw_box(
        f,
        window_x,
        window_y,
        window_w,
        window_h,
        crossterm::style::Color::Grey,
    );

    // Divider between reels
    let inner_x = window_x + 1;
    let inner_y = window_y + 1;
    let inner_w = window_w - 2;
    let inner_h = window_h - 2;

    let reel_w = inner_w / (REELS as u16);
    for i in 1..REELS {
        let x = inner_x + reel_w * (i as u16);
        for yy in inner_y..(inner_y + inner_h) {
            let pipe = if yy % 2 == 0 { '║' } else { '│' };
            f.set(x, yy, pipe, crossterm::style::Color::DarkGrey, Attribute::Reset);
        }
    }

    // Render reels (3 rows visible) with speed-based smear
    for i in 0..REELS {
        let rx = inner_x + reel_w * (i as u16);
        let rw = reel_w;
        render_reel(
            f,
            rx,
            inner_y,
            rw,
            inner_h,
            &reels[i],
            spinning,
            last_win,
            i,
            time_s,
        );
    }

    // Payline highlight across center row.
    let line_y = inner_y + (inner_h / 2);
    let line_color = match last_win.kind {
        WinKind::ThreeMatch => crossterm::style::Color::Yellow,
        WinKind::TwoMatch => crossterm::style::Color::Green,
        WinKind::None => crossterm::style::Color::DarkGrey,
    };
    let blink = ((time_s * 9.0) as i32 % 2) == 0;
    let reel_centers = [
        inner_x + (reel_w / 2),
        inner_x + reel_w + (reel_w / 2),
        inner_x + reel_w * 2 + (reel_w / 2),
    ];
    for xx in (inner_x + 1)..(inner_x + inner_w - 1) {
        if reel_centers
            .iter()
            .any(|&cx| xx >= cx.saturating_sub(1) && xx <= cx.saturating_add(1))
        {
            continue;
        }
        let ch = if spinning {
            if xx % 2 == 0 { '─' } else { ' ' }
        } else if blink && !matches!(last_win.kind, WinKind::None) {
            '═'
        } else {
            '─'
        };
        f.set(
            xx,
            line_y,
            ch,
            line_color,
            if blink { Attribute::Bold } else { Attribute::Reset },
        );
    }
    f.set(inner_x, line_y, '◀', line_color, Attribute::Bold);
    f.set(inner_x + inner_w - 1, line_y, '▶', line_color, Attribute::Bold);

    // Status bar
    let status_y = top + box_h + 2;
    f.write_str(
        left,
        status_y,
        &format!("Credits: {:>6}    Bet: {:>4}", credits, bet),
        crossterm::style::Color::White,
        Attribute::Bold,
    );
    f.write_str(
        left + 32,
        status_y,
        &format!("Seed: {}", seed),
        crossterm::style::Color::DarkGrey,
        Attribute::Reset,
    );

    // Win message + payout table hint
    let msg_y = status_y + 1;
    match last_win.kind {
        WinKind::None => {
            f.write_str(
                left,
                msg_y,
                if spinning { "Spinning..." } else { "Ready." },
                crossterm::style::Color::DarkGrey,
                Attribute::Reset,
            );
        }
        WinKind::TwoMatch => {
            f.write_str(
                left,
                msg_y,
                &format!("Two match!  +{}", last_win.payout),
                crossterm::style::Color::Yellow,
                Attribute::Bold,
            );
        }
        WinKind::ThreeMatch => {
            f.write_str(
                left,
                msg_y,
                &format!("JACKPOT!  +{}", last_win.payout),
                crossterm::style::Color::Green,
                Attribute::Bold,
            );
        }
    }

    f.write_str(
        left,
        msg_y + 1,
        "Payouts: 3-match = bet * (rarer symbol => higher). 2-match = bet*2.",
        crossterm::style::Color::DarkGrey,
        Attribute::Reset,
    );
}

fn render_reel(
    f: &mut Frame,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    reel: &Reel,
    spinning: bool,
    last_win: WinInfo,
    reel_i: usize,
    time_s: f32,
) {
    let speed = reel.speed.abs();
    let smear_level = if spinning {
        if speed > 28.0 {
            3
        } else if speed > 20.0 {
            2
        } else if speed > 12.0 {
            1
        } else {
            0
        }
    } else {
        0
    };

    let cx = x + (w / 2);
    let top_phase = reel.phase.floor() as i32;
    let frac = reel.phase - reel.phase.floor();
    let n = SYMBOLS.len() as i32;

    // Curved reel bed: narrower at top/bottom, wider at center.
    for yy in y..(y + h) {
        let rel = ((yy - y) as f32 + 0.5) / (h as f32) * 2.0 - 1.0;
        let inset = ((rel.abs() * rel.abs()) * ((w as f32) * 0.24)) as u16;
        let lx = x + inset;
        let rx = (x + w).saturating_sub(inset + 1);

        for xx in x..(x + w) {
            let in_body = xx >= lx && xx <= rx;
            if in_body {
                let band = if rel.abs() < 0.22 {
                    ' '
                } else if rel.abs() < 0.55 {
                    '░'
                } else {
                    '▒'
                };
                let fg = if rel.abs() < 0.28 {
                    crossterm::style::Color::Grey
                } else {
                    crossterm::style::Color::DarkGrey
                };
                f.set(xx, yy, band, fg, Attribute::Reset);
            } else if xx == lx.saturating_sub(1) || xx == rx.saturating_add(1) {
                f.set(
                    xx,
                    yy,
                    '▕',
                    crossterm::style::Color::DarkGrey,
                    Attribute::Reset,
                );
            }
        }
    }

    // Draw symbols with vertical interpolation while keeping logical middle symbol on payline.
    let mid_yf = y as f32 + (h as f32 / 2.0);
    let mid_idx = (top_phase + 1).rem_euclid(n); // equivalent to symbol_at(reel, 1)
    for k in -3..=3 {
        let yyf = mid_yf + (k as f32 - frac);
        if yyf < y as f32 || yyf > (y + h - 1) as f32 {
            continue;
        }
        let yy = yyf.round() as u16;
        let idx = (mid_idx + k).rem_euclid(n) as usize;
        let sym = SYMBOLS[idx];
        let dist = (yyf - mid_yf).abs();

        let color = if dist < 0.35 {
            symbol_color(idx)
        } else if dist < 1.25 {
            crossterm::style::Color::White
        } else if dist < 2.25 {
            crossterm::style::Color::Grey
        } else {
            crossterm::style::Color::DarkGrey
        };
        let attr = if dist < 0.35 {
            Attribute::Bold
        } else {
            Attribute::Reset
        };
        f.set(cx, yy, sym, color, attr);
    }

    // Motion streak overlay while spinning.
    if smear_level > 0 {
        let streak_char = SMEAR[min(smear_level, SMEAR.len() - 1)];
        for yy in y..(y + h) {
            if (yy + reel_i as u16) % 2 == 0 {
                f.set(
                    cx,
                    yy,
                    streak_char,
                    crossterm::style::Color::DarkGrey,
                    Attribute::Reset,
                );
            }
        }
    }

    // Win highlight (middle row) if not spinning and win exists
    let win_mid = matches!(last_win.kind, WinKind::TwoMatch | WinKind::ThreeMatch)
        && !spinning
        && last_win.line[reel_i] == symbol_at(reel, 1);

    // Pulse ring around winning center symbol.
    if win_mid {
        let pulse_on = ((time_s * 8.0) as i32 % 2) == 0;
        let mid_y = y + (h / 2);
        let ring_color = if pulse_on {
            crossterm::style::Color::Yellow
        } else {
            crossterm::style::Color::Green
        };
        f.set(cx.saturating_sub(2), mid_y, '❮', ring_color, Attribute::Bold);
        f.set(cx.saturating_add(2), mid_y, '❯', ring_color, Attribute::Bold);
        f.set(cx, mid_y, SYMBOLS[symbol_at(reel, 1)], ring_color, Attribute::Bold);
    } else {
        // center lane markers
        let mid_y = y + (h / 2);
        f.set(
            cx.saturating_sub(2),
            mid_y,
            '‹',
            crossterm::style::Color::DarkGrey,
            Attribute::Reset,
        );
        f.set(
            cx.saturating_add(2),
            mid_y,
            '›',
            crossterm::style::Color::DarkGrey,
            Attribute::Reset,
        );
    }
}

fn symbol_color(idx: usize) -> crossterm::style::Color {
    match SYMBOLS[idx] {
        '7' => crossterm::style::Color::Red,
        '♦' | '♥' => crossterm::style::Color::Magenta,
        '♣' | '♠' => crossterm::style::Color::Cyan,
        '★' => crossterm::style::Color::Yellow,
        '✿' => crossterm::style::Color::Blue,
        '☘' => crossterm::style::Color::Green,
        '◈' => crossterm::style::Color::White,
        _ => crossterm::style::Color::Grey,
    }
}

fn draw_box(f: &mut Frame, x: u16, y: u16, w: u16, h: u16, color: crossterm::style::Color) {
    if w < 2 || h < 2 {
        return;
    }
    let x2 = x + w - 1;
    let y2 = y + h - 1;

    f.set(x, y, '┌', color, Attribute::Reset);
    f.set(x2, y, '┐', color, Attribute::Reset);
    f.set(x, y2, '└', color, Attribute::Reset);
    f.set(x2, y2, '┘', color, Attribute::Reset);

    for xx in (x + 1)..x2 {
        f.set(xx, y, '─', color, Attribute::Reset);
        f.set(xx, y2, '─', color, Attribute::Reset);
    }
    for yy in (y + 1)..y2 {
        f.set(x, yy, '│', color, Attribute::Reset);
        f.set(x2, yy, '│', color, Attribute::Reset);
    }
}

fn flush_diff(stdout: &mut io::Stdout, back: &Frame, front: &mut Frame) -> Result<()> {
    // Ensure same size
    if back.w != front.w || back.h != front.h {
        *front = Frame::new(back.w, back.h);
        front.clear('\0', crossterm::style::Color::White, Attribute::Reset);
    }

    queue!(stdout, BeginSynchronizedUpdate)?;
    let mut last_fg = None;
    let mut last_attr = None;

    for y in 0..back.h {
        for x in 0..back.w {
            let i = back.idx(x, y);
            let b = back.cells[i];
            let fcell = front.cells[i];

            if b.ch == fcell.ch && b.fg == fcell.fg && b.attr == fcell.attr {
                continue;
            }

            queue!(stdout, cursor::MoveTo(x, y))?;

            if last_fg != Some(b.fg) {
                queue!(stdout, SetForegroundColor(b.fg))?;
                last_fg = Some(b.fg);
            }
            if last_attr != Some(b.attr) {
                queue!(stdout, SetAttribute(b.attr))?;
                last_attr = Some(b.attr);
            }

            queue!(stdout, Print(b.ch))?;
            front.cells[i] = b;
        }
    }

    queue!(stdout, SetAttribute(Attribute::Reset), ResetColor, EndSynchronizedUpdate)?;
    stdout.flush()?;
    Ok(())
}
