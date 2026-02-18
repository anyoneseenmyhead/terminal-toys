use std::cmp::{max, min};
use std::collections::VecDeque;
use std::env;
use std::io::{stdout, Stdout, Write};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::style::{Color, Print, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{
    self, disable_raw_mode, enable_raw_mode, BeginSynchronizedUpdate, DisableLineWrap,
    EnableLineWrap, EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, queue};
use rand::seq::SliceRandom;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

const FPS_CAP: f32 = 60.0;
const DT_CLAMP: f32 = 0.05;
const CARD_W: i32 = 9;
const CARD_H: i32 = 5;
const CARD_GAP_X: i32 = 2;
const MIN_BET: i64 = 10;
const BET_STEP: i64 = 10;
const STARTING_CHIPS: i64 = 500;
const NUM_DECKS: usize = 6;
const PENETRATION_NUM: usize = 25;
const PENETRATION_DEN: usize = 100;
const DEAL_DURATION: f32 = 0.28;
const FLIP_DURATION: f32 = 0.3;
const FLASH_DURATION: f32 = 1.5;
const PAYOUT_DELAY: f32 = 2.0;
const MAX_SPLIT_HANDS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Suit {
    Spades,
    Hearts,
    Diamonds,
    Clubs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    A,
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    Ten,
    J,
    Q,
    K,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Card {
    rank: Rank,
    suit: Suit,
}

fn rank_str(rank: Rank) -> &'static str {
    match rank {
        Rank::A => "A",
        Rank::Two => "2",
        Rank::Three => "3",
        Rank::Four => "4",
        Rank::Five => "5",
        Rank::Six => "6",
        Rank::Seven => "7",
        Rank::Eight => "8",
        Rank::Nine => "9",
        Rank::Ten => "10",
        Rank::J => "J",
        Rank::Q => "Q",
        Rank::K => "K",
    }
}

fn suit_glyph(suit: Suit) -> char {
    match suit {
        Suit::Spades => '♠',
        Suit::Hearts => '♥',
        Suit::Diamonds => '♦',
        Suit::Clubs => '♣',
    }
}

fn card_value(rank: Rank) -> u8 {
    match rank {
        Rank::A => 1,
        Rank::Two => 2,
        Rank::Three => 3,
        Rank::Four => 4,
        Rank::Five => 5,
        Rank::Six => 6,
        Rank::Seven => 7,
        Rank::Eight => 8,
        Rank::Nine => 9,
        Rank::Ten | Rank::J | Rank::Q | Rank::K => 10,
    }
}

#[derive(Clone, Debug)]
struct Hand {
    cards: Vec<Card>,
    stood: bool,
    doubled: bool,
    busted: bool,
}

impl Hand {
    fn new() -> Self {
        Self {
            cards: Vec::new(),
            stood: false,
            doubled: false,
            busted: false,
        }
    }

    fn totals(&self) -> (u8, bool) {
        totals(self)
    }

    fn is_blackjack(&self) -> bool {
        is_blackjack(self)
    }
}

fn totals(hand: &Hand) -> (u8, bool) {
    let mut sum: u8 = 0;
    let mut aces = 0u8;
    for c in &hand.cards {
        sum = sum.saturating_add(card_value(c.rank));
        if c.rank == Rank::A {
            aces += 1;
        }
    }
    let mut best = sum;
    let mut soft = false;
    for _ in 0..aces {
        if best + 10 <= 21 {
            best += 10;
            soft = true;
        }
    }
    (best, soft)
}

fn is_blackjack(hand: &Hand) -> bool {
    hand.cards.len() == 2 && hand.totals().0 == 21
}

#[derive(Clone)]
struct Shoe {
    cards: Vec<Card>,
    decks: usize,
    cut: usize,
}

impl Shoe {
    fn new(decks: usize, rng: &mut ChaCha8Rng) -> Self {
        let mut cards = Vec::with_capacity(decks * 52);
        let suits = [Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs];
        let ranks = [
            Rank::A,
            Rank::Two,
            Rank::Three,
            Rank::Four,
            Rank::Five,
            Rank::Six,
            Rank::Seven,
            Rank::Eight,
            Rank::Nine,
            Rank::Ten,
            Rank::J,
            Rank::Q,
            Rank::K,
        ];
        for _ in 0..decks {
            for suit in suits {
                for rank in ranks {
                    cards.push(Card { rank, suit });
                }
            }
        }
        cards.shuffle(rng);
        let cut = cards.len() * PENETRATION_NUM / PENETRATION_DEN;
        Self { cards, decks, cut }
    }

    fn draw(&mut self) -> Card {
        self.cards.pop().expect("shoe exhausted")
    }

    fn needs_shuffle(&self) -> bool {
        self.cards.len() < self.cut
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Betting,
    Dealing,
    PlayerTurn,
    DealerTurn,
    Payout,
    Shuffle,
}

#[derive(Clone, Copy)]
struct RulesConfig {
    dealer_hits_soft_17: bool,
    blackjack_payout_num: i64,
    blackjack_payout_den: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Vec2 {
    x: i32,
    y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

#[derive(Clone, Debug)]
enum DealDest {
    Player { hand: usize },
    DealerUp,
    DealerDown,
}

#[derive(Clone, Debug)]
enum AnimEvent {
    Deal {
        card: Card,
        from: Vec2,
        to: Vec2,
        dest: DealDest,
        duration: f32,
    },
    FlipHole {
        at: Vec2,
        duration: f32,
    },
    Flash {
        rect: Rect,
        duration: f32,
    },
    ChipDelta {
        amount: i64,
        at: Vec2,
        duration: f32,
    },
}

#[derive(Clone, Debug)]
struct ActiveAnim {
    event: AnimEvent,
    elapsed: f32,
}

#[derive(Clone, Debug)]
struct AnimState {
    now: f32,
    queue: VecDeque<AnimEvent>,
    active: Option<ActiveAnim>,
}

impl AnimState {
    fn new() -> Self {
        Self {
            now: 0.0,
            queue: VecDeque::new(),
            active: None,
        }
    }

    fn enqueue(&mut self, event: AnimEvent) {
        self.queue.push_back(event);
    }

    fn is_busy(&self) -> bool {
        self.active.is_some() || !self.queue.is_empty()
    }
}

#[derive(Clone, Copy, Debug)]
struct Layout {
    table: Rect,
    shoe: Vec2,
    dealer_origin: Vec2,
    player_origin: Vec2,
    bar_y: i32,
}

impl Layout {
    fn compute(w: u16, h: u16) -> Self {
        let term_w = w as i32;
        let term_h = h as i32;
        let table_w = min(term_w - 2, 96).max(32);
        let table_h = min(term_h - 2, 28).max(18);
        let table_x = (term_w - table_w) / 2;
        let table_y = (term_h - table_h) / 2;
        let table = Rect {
            x: table_x,
            y: table_y,
            w: table_w,
            h: table_h,
        };
        let shoe = Vec2 {
            x: table.x + 3,
            y: table.y + table.h / 2 - CARD_H / 2,
        };
        let dealer_origin = Vec2 {
            x: table.x + 14,
            y: table.y + 3,
        };
        let player_origin = Vec2 {
            x: table.x + 14,
            y: table.y + table.h - CARD_H - 5,
        };
        let bar_y = table.y + table.h - 2;
        Self {
            table,
            shoe,
            dealer_origin,
            player_origin,
            bar_y,
        }
    }

    fn dealer_card_pos(&self, idx: usize) -> Vec2 {
        Vec2 {
            x: self.dealer_origin.x + idx as i32 * CARD_GAP_X,
            y: self.dealer_origin.y,
        }
    }

    fn player_card_pos(&self, hand_idx: usize, card_idx: usize) -> Vec2 {
        let hand_stride = CARD_W + CARD_GAP_X + 20;
        Vec2 {
            x: self.player_origin.x + hand_idx as i32 * hand_stride + card_idx as i32 * CARD_GAP_X,
            y: self.player_origin.y,
        }
    }

    fn player_hand_rect(&self, hand_idx: usize) -> Rect {
        let hand_stride = CARD_W + CARD_GAP_X + 20;
        Rect {
            x: self.player_origin.x + hand_idx as i32 * hand_stride - 2,
            y: self.player_origin.y - 1,
            w: CARD_W + 12,
            h: CARD_H + 3,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Cell {
    ch: char,
    fg: Color,
    bg: Color,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::White,
            bg: Color::Black,
        }
    }
}

struct FrameBuffer {
    w: u16,
    h: u16,
    cells: Vec<Cell>,
}

impl FrameBuffer {
    fn new(w: u16, h: u16) -> Self {
        Self {
            w,
            h,
            cells: vec![Cell::default(); w as usize * h as usize],
        }
    }

    fn clear(&mut self, bg: Color) {
        for c in &mut self.cells {
            c.ch = ' ';
            c.fg = Color::White;
            c.bg = bg;
        }
    }

    fn set(&mut self, x: i32, y: i32, ch: char, fg: Color, bg: Color) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let idx = y as usize * self.w as usize + x as usize;
        self.cells[idx] = Cell { ch, fg, bg };
    }

    fn text(&mut self, x: i32, y: i32, s: &str, fg: Color, bg: Color) {
        for (i, ch) in s.chars().enumerate() {
            self.set(x + i as i32, y, ch, fg, bg);
        }
    }

    fn hline(&mut self, x: i32, y: i32, w: i32, ch: char, fg: Color, bg: Color) {
        for dx in 0..w {
            self.set(x + dx, y, ch, fg, bg);
        }
    }

    fn rect_border(&mut self, r: Rect, fg: Color, bg: Color) {
        if r.w < 2 || r.h < 2 {
            return;
        }
        self.set(r.x, r.y, '┌', fg, bg);
        self.set(r.x + r.w - 1, r.y, '┐', fg, bg);
        self.set(r.x, r.y + r.h - 1, '└', fg, bg);
        self.set(r.x + r.w - 1, r.y + r.h - 1, '┘', fg, bg);
        self.hline(r.x + 1, r.y, r.w - 2, '─', fg, bg);
        self.hline(r.x + 1, r.y + r.h - 1, r.w - 2, '─', fg, bg);
        for y in r.y + 1..r.y + r.h - 1 {
            self.set(r.x, y, '│', fg, bg);
            self.set(r.x + r.w - 1, y, '│', fg, bg);
        }
    }
}

#[derive(Clone, Copy)]
enum DealerReveal {
    Hidden,
    Revealed,
}

struct GameState {
    phase: Phase,
    shoe: Shoe,
    dealer: Hand,
    players: Vec<Hand>,
    wagers: Vec<i64>,
    settled_blackjack: Vec<bool>,
    active_hand: usize,
    chips: i64,
    bet: i64,
    message: String,
    rng: ChaCha8Rng,
    rules: RulesConfig,
    anim: AnimState,
    dealer_reveal: DealerReveal,
    pending_stand_after_deal: Option<usize>,
    pending_natural_resolution: bool,
    payout_timer: f32,
}

impl GameState {
    fn new(seed: u64) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let shoe = Shoe::new(NUM_DECKS, &mut rng);
        Self {
            phase: Phase::Betting,
            shoe,
            dealer: Hand::new(),
            players: vec![Hand::new()],
            wagers: vec![MIN_BET],
            settled_blackjack: vec![false],
            active_hand: 0,
            chips: STARTING_CHIPS,
            bet: MIN_BET,
            message: String::from("Adjust bet and press Enter to deal"),
            rng,
            rules: RulesConfig {
                dealer_hits_soft_17: true,
                blackjack_payout_num: 3,
                blackjack_payout_den: 2,
            },
            anim: AnimState::new(),
            dealer_reveal: DealerReveal::Hidden,
            pending_stand_after_deal: None,
            pending_natural_resolution: false,
            payout_timer: 0.0,
        }
    }

    fn reset_round_hands(&mut self) {
        self.dealer = Hand::new();
        self.players.clear();
        self.players.push(Hand::new());
        self.wagers.clear();
        self.wagers.push(self.bet);
        self.settled_blackjack.clear();
        self.settled_blackjack.push(false);
        self.active_hand = 0;
        self.dealer_reveal = DealerReveal::Hidden;
        self.pending_stand_after_deal = None;
        self.pending_natural_resolution = false;
    }

    fn can_double(&self, hand_idx: usize) -> bool {
        if self.phase != Phase::PlayerTurn || self.anim.is_busy() {
            return false;
        }
        let hand = &self.players[hand_idx];
        hand.cards.len() == 2 && !hand.stood && !hand.busted && self.chips >= self.wagers[hand_idx]
    }

    fn can_split(&self, hand_idx: usize) -> bool {
        if self.phase != Phase::PlayerTurn || self.anim.is_busy() {
            return false;
        }
        if self.players.len() >= MAX_SPLIT_HANDS {
            return false;
        }
        let hand = &self.players[hand_idx];
        if hand.cards.len() != 2 || self.chips < self.wagers[hand_idx] {
            return false;
        }
        hand.cards[0].rank == hand.cards[1].rank
    }

    fn hand_label(&self, idx: usize) -> String {
        let hand = &self.players[idx];
        let (tot, soft) = hand.totals();
        let soft_str = if soft { " (soft)" } else { "" };
        format!("H{}: {}{}", idx + 1, tot, soft_str)
    }

    fn start_round(&mut self, layout: Layout) {
        if self.bet > self.chips {
            self.message = "Not enough chips".to_string();
            return;
        }
        self.reset_round_hands();
        self.chips -= self.bet;
        self.phase = Phase::Dealing;
        self.message = "Dealing...".to_string();
        self.enqueue_initial_deal(layout);
    }

    fn enqueue_initial_deal(&mut self, layout: Layout) {
        let c1 = self.shoe.draw();
        self.anim.enqueue(AnimEvent::Deal {
            card: c1,
            from: layout.shoe,
            to: layout.player_card_pos(0, 0),
            dest: DealDest::Player { hand: 0 },
            duration: DEAL_DURATION,
        });

        let c2 = self.shoe.draw();
        self.anim.enqueue(AnimEvent::Deal {
            card: c2,
            from: layout.shoe,
            to: layout.dealer_card_pos(0),
            dest: DealDest::DealerUp,
            duration: DEAL_DURATION,
        });

        let c3 = self.shoe.draw();
        self.anim.enqueue(AnimEvent::Deal {
            card: c3,
            from: layout.shoe,
            to: layout.player_card_pos(0, 1),
            dest: DealDest::Player { hand: 0 },
            duration: DEAL_DURATION,
        });

        let c4 = self.shoe.draw();
        self.anim.enqueue(AnimEvent::Deal {
            card: c4,
            from: layout.shoe,
            to: layout.dealer_card_pos(1),
            dest: DealDest::DealerDown,
            duration: DEAL_DURATION,
        });
    }

    fn enqueue_deal_to_player(&mut self, hand_idx: usize, layout: Layout) {
        let card = self.shoe.draw();
        let card_idx = self.players[hand_idx].cards.len();
        self.anim.enqueue(AnimEvent::Deal {
            card,
            from: layout.shoe,
            to: layout.player_card_pos(hand_idx, card_idx),
            dest: DealDest::Player { hand: hand_idx },
            duration: DEAL_DURATION,
        });
    }

    fn enqueue_deal_to_dealer(&mut self, layout: Layout) {
        let card = self.shoe.draw();
        let card_idx = self.dealer.cards.len();
        self.anim.enqueue(AnimEvent::Deal {
            card,
            from: layout.shoe,
            to: layout.dealer_card_pos(card_idx),
            dest: DealDest::DealerUp,
            duration: DEAL_DURATION,
        });
    }

    fn enqueue_flip_hole(&mut self, layout: Layout) {
        self.anim.enqueue(AnimEvent::FlipHole {
            at: layout.dealer_card_pos(1),
            duration: FLIP_DURATION,
        });
    }

    fn progress_active_hand(&mut self) {
        while self.active_hand < self.players.len() {
            let h = &self.players[self.active_hand];
            if h.stood || h.busted {
                self.active_hand += 1;
            } else {
                break;
            }
        }
        if self.active_hand >= self.players.len() {
            self.phase = Phase::DealerTurn;
        }
    }

    fn handle_player_action(&mut self, key: char, layout: Layout) {
        if self.phase != Phase::PlayerTurn || self.anim.is_busy() {
            return;
        }
        let idx = self.active_hand;
        if idx >= self.players.len() {
            return;
        }
        match key {
            'h' => {
                self.enqueue_deal_to_player(idx, layout);
                self.message = format!("Hit hand {}", idx + 1);
            }
            's' => {
                self.players[idx].stood = true;
                self.message = format!("Stand hand {}", idx + 1);
                self.progress_active_hand();
            }
            'd' => {
                if self.can_double(idx) {
                    let add = self.wagers[idx];
                    self.chips -= add;
                    self.wagers[idx] += add;
                    self.players[idx].doubled = true;
                    self.pending_stand_after_deal = Some(idx);
                    self.enqueue_deal_to_player(idx, layout);
                    self.message = format!("Double hand {}", idx + 1);
                } else {
                    self.message = "Double not allowed".to_string();
                }
            }
            'p' => {
                if self.can_split(idx) {
                    self.chips -= self.wagers[idx];
                    let mut second = Hand::new();
                    let card = self.players[idx].cards.pop().expect("split card missing");
                    second.cards.push(card);
                    self.players.insert(idx + 1, second);
                    self.wagers.insert(idx + 1, self.wagers[idx]);
                    self.settled_blackjack.insert(idx + 1, false);
                    self.enqueue_deal_to_player(idx, layout);
                    self.enqueue_deal_to_player(idx + 1, layout);
                    self.message = format!("Split hand {}", idx + 1);
                } else {
                    self.message = "Split not allowed".to_string();
                }
            }
            _ => {}
        }
    }

    fn post_deal_checks(&mut self) {
        if let Some(hand_idx) = self.pending_stand_after_deal.take() {
            if let Some(hand) = self.players.get_mut(hand_idx) {
                let (tot, _) = hand.totals();
                hand.busted = tot > 21;
                hand.stood = true;
            }
            self.progress_active_hand();
            return;
        }
        if self.phase == Phase::PlayerTurn {
            if self.active_hand < self.players.len() {
                let hand = &mut self.players[self.active_hand];
                let (tot, _) = hand.totals();
                hand.busted = tot > 21;
                if hand.busted {
                    hand.stood = true;
                    self.message = format!("Hand {} busts", self.active_hand + 1);
                    self.progress_active_hand();
                }
            }
        }
    }

    fn on_dealing_finished(&mut self, layout: Layout) {
        if self.phase != Phase::Dealing {
            return;
        }
        if self.pending_natural_resolution {
            self.resolve_natural_blackjack(layout);
            return;
        }
        if self.players[0].is_blackjack() {
            self.pending_natural_resolution = true;
            self.enqueue_flip_hole(layout);
            self.message = "Natural blackjack check".to_string();
        } else {
            self.phase = Phase::PlayerTurn;
            self.active_hand = 0;
            self.message = "Player turn: h/s/d/p".to_string();
        }
    }

    fn resolve_natural_blackjack(&mut self, layout: Layout) {
        self.pending_natural_resolution = false;
        self.dealer_reveal = DealerReveal::Revealed;
        let dealer_bj = self.dealer.is_blackjack();
        if dealer_bj {
            self.chips += self.wagers[0];
            self.message = "Push: both blackjack".to_string();
        } else {
            let w = self.wagers[0];
            let bonus = w * self.rules.blackjack_payout_num / self.rules.blackjack_payout_den;
            self.chips += w + bonus;
            self.settled_blackjack[0] = true;
            self.message = "Blackjack pays 3:2".to_string();
            self.anim.enqueue(AnimEvent::Flash {
                rect: layout.player_hand_rect(0),
                duration: FLASH_DURATION,
            });
            self.anim.enqueue(AnimEvent::ChipDelta {
                amount: w + bonus,
                at: Vec2 {
                    x: layout.player_hand_rect(0).x,
                    y: layout.player_hand_rect(0).y - 1,
                },
                duration: 1.0,
            });
        }
        self.enter_payout();
    }

    fn start_dealer_turn(&mut self, layout: Layout) {
        self.phase = Phase::DealerTurn;
        if self.dealer.cards.len() < 2 {
            return;
        }
        if matches!(self.dealer_reveal, DealerReveal::Hidden) {
            self.enqueue_flip_hole(layout);
        }
        self.message = "Dealer turn".to_string();
    }

    fn update_dealer_policy(&mut self, layout: Layout) {
        if self.phase != Phase::DealerTurn || self.anim.is_busy() {
            return;
        }
        self.dealer_reveal = DealerReveal::Revealed;
        let (tot, soft) = self.dealer.totals();
        let hit = if tot < 17 {
            true
        } else {
            tot == 17 && soft && self.rules.dealer_hits_soft_17
        };
        if hit {
            self.enqueue_deal_to_dealer(layout);
            self.message = "Dealer hits".to_string();
        } else {
            self.resolve_standard_payout(layout);
            self.enter_payout();
        }
    }

    fn enter_payout(&mut self) {
        self.phase = Phase::Payout;
        self.payout_timer = PAYOUT_DELAY;
    }

    fn resolve_standard_payout(&mut self, layout: Layout) {
        let (dealer_total, _) = self.dealer.totals();
        let dealer_bust = dealer_total > 21;
        let mut wins = 0usize;
        let mut pushes = 0usize;
        let mut losses = 0usize;

        for i in 0..self.players.len() {
            if self.settled_blackjack[i] {
                continue;
            }
            let hand = &self.players[i];
            let (pt, _) = hand.totals();
            let wager = self.wagers[i];

            if hand.busted || pt > 21 {
                losses += 1;
                continue;
            }

            let result = if dealer_bust {
                1
            } else if pt > dealer_total {
                1
            } else if pt < dealer_total {
                -1
            } else {
                0
            };

            match result {
                1 => {
                    wins += 1;
                    self.chips += wager * 2;
                    self.anim.enqueue(AnimEvent::Flash {
                        rect: layout.player_hand_rect(i),
                        duration: FLASH_DURATION,
                    });
                    self.anim.enqueue(AnimEvent::ChipDelta {
                        amount: wager,
                        at: Vec2 {
                            x: layout.player_hand_rect(i).x,
                            y: layout.player_hand_rect(i).y - 1,
                        },
                        duration: 1.0,
                    });
                }
                0 => {
                    pushes += 1;
                    self.chips += wager;
                    self.anim.enqueue(AnimEvent::ChipDelta {
                        amount: 0,
                        at: Vec2 {
                            x: layout.player_hand_rect(i).x,
                            y: layout.player_hand_rect(i).y - 1,
                        },
                        duration: 0.85,
                    });
                }
                _ => {
                    losses += 1;
                    self.anim.enqueue(AnimEvent::ChipDelta {
                        amount: -wager,
                        at: Vec2 {
                            x: layout.player_hand_rect(i).x,
                            y: layout.player_hand_rect(i).y - 1,
                        },
                        duration: 1.0,
                    });
                }
            }
        }

        self.message = format!(
            "Payout: {} win / {} push / {} lose",
            wins, pushes, losses
        );
    }

    fn finish_round(&mut self) {
        if self.shoe.needs_shuffle() {
            self.phase = Phase::Shuffle;
            self.message = "SHUFFLING...".to_string();
            self.payout_timer = 0.8;
        } else {
            self.phase = Phase::Betting;
            self.bet = self.bet.min(self.chips.max(MIN_BET));
            if self.chips <= 0 {
                self.chips = STARTING_CHIPS;
                self.message = "Out of chips. Bankroll reset.".to_string();
            } else {
                self.message = "Adjust bet and press Enter to deal".to_string();
            }
        }
    }

    fn reset_all(&mut self, seed: u64) {
        *self = GameState::new(seed);
    }
}

fn lerp(a: i32, b: i32, t: f32) -> i32 {
    (a as f32 + (b - a) as f32 * t).round() as i32
}

fn draw_card_sprite(
    fb: &mut FrameBuffer,
    pos: Vec2,
    card: Option<Card>,
    face_down: bool,
    highlight: bool,
    thin_width: Option<i32>,
    bg: Color,
) {
    let width = thin_width.unwrap_or(CARD_W).max(1);
    let fg = if highlight { Color::Yellow } else { Color::White };

    if width == CARD_W {
        let rect = Rect {
            x: pos.x,
            y: pos.y,
            w: CARD_W,
            h: CARD_H,
        };
        fb.rect_border(rect, fg, bg);
        // Paint a solid face so overlapping cards do not show through.
        for y in 1..CARD_H - 1 {
            for x in 1..CARD_W - 1 {
                fb.set(pos.x + x, pos.y + y, ' ', Color::White, bg);
            }
        }
        if face_down {
            let pattern = ['░', '▒', '▓', '▒'];
            for y in 1..CARD_H - 1 {
                for x in 1..CARD_W - 1 {
                    let ch = pattern[((x + y) as usize) % pattern.len()];
                    fb.set(pos.x + x, pos.y + y, ch, Color::Blue, bg);
                }
            }
        } else if let Some(card) = card {
            let rank = rank_str(card.rank);
            let suit = suit_glyph(card.suit);
            let suit_color = match card.suit {
                Suit::Hearts | Suit::Diamonds => Color::Red,
                _ => Color::White,
            };
            fb.text(pos.x + 1, pos.y + 1, rank, suit_color, bg);
            fb.set(pos.x + 4, pos.y + 2, suit, suit_color, bg);
            let right_x = pos.x + CARD_W - 1 - rank.chars().count() as i32 - 1;
            fb.text(right_x, pos.y + 3, rank, suit_color, bg);
        }
    } else {
        let x = pos.x + (CARD_W - width) / 2;
        for dy in 0..CARD_H {
            for dx in 0..width {
                let ch = if dx == 0 || dx == width - 1 { '│' } else { ' ' };
                let is_top = dy == 0;
                let is_bottom = dy == CARD_H - 1;
                let ch = if is_top || is_bottom {
                    if dx == 0 || dx == width - 1 {
                        '┼'
                    } else {
                        '─'
                    }
                } else {
                    ch
                };
                fb.set(x + dx, pos.y + dy, ch, fg, bg);
            }
        }
    }
}

fn draw_hand_total(fb: &mut FrameBuffer, pos: Vec2, hand: &Hand, fg: Color, bg: Color) {
    let (tot, soft) = hand.totals();
    let txt = if soft {
        format!("{} (soft)", tot)
    } else {
        format!("{}", tot)
    };
    fb.text(pos.x, pos.y, &txt, fg, bg);
}

fn render_game(fb: &mut FrameBuffer, gs: &GameState, layout: Layout) {
    let table_bg = Color::DarkGreen;
    fb.clear(table_bg);

    fb.rect_border(layout.table, Color::DarkGrey, table_bg);
    let title = "BLACKJACK";
    let title_x = layout.table.x + layout.table.w - 2 - title.chars().count() as i32;
    fb.text(title_x, layout.table.y + 1, title, Color::White, table_bg);
    draw_card_sprite(fb, layout.shoe, None, true, false, None, table_bg);
    fb.text(layout.shoe.x, layout.shoe.y - 1, "SHOE", Color::Grey, table_bg);

    fb.text(
        layout.player_origin.x,
        layout.dealer_origin.y - 1,
        "Dealer",
        Color::White,
        table_bg,
    );

    for (i, card) in gs.dealer.cards.iter().enumerate() {
        let pos = layout.dealer_card_pos(i);
        let hidden = i == 1 && matches!(gs.dealer_reveal, DealerReveal::Hidden);
        draw_card_sprite(fb, pos, Some(*card), hidden, false, None, table_bg);
    }

    if gs.dealer.cards.is_empty() {
        fb.text(layout.dealer_origin.x, layout.dealer_origin.y + 2, "...", Color::Grey, table_bg);
    }

    if !gs.dealer.cards.is_empty() {
        if matches!(gs.dealer_reveal, DealerReveal::Hidden) && gs.dealer.cards.len() >= 2 {
            let up_val = card_value(gs.dealer.cards[0].rank);
            fb.text(
                layout.dealer_origin.x + 22,
                layout.dealer_origin.y + 2,
                &format!("Total: {}+?", up_val),
                Color::White,
                table_bg,
            );
        } else {
            fb.text(
                layout.dealer_origin.x + 22,
                layout.dealer_origin.y + 2,
                "Total:",
                Color::White,
                table_bg,
            );
            draw_hand_total(
                fb,
                Vec2 {
                    x: layout.dealer_origin.x + 29,
                    y: layout.dealer_origin.y + 2,
                },
                &gs.dealer,
                Color::White,
                table_bg,
            );
        }
    }

    fb.text(
        layout.player_origin.x,
        layout.player_origin.y - 1,
        "Player",
        Color::White,
        table_bg,
    );

    for (h_idx, hand) in gs.players.iter().enumerate() {
        let active = gs.phase == Phase::PlayerTurn && h_idx == gs.active_hand;
        for (c_idx, card) in hand.cards.iter().enumerate() {
            draw_card_sprite(
                fb,
                layout.player_card_pos(h_idx, c_idx),
                Some(*card),
                false,
                active,
                None,
                table_bg,
            );
        }

        let label = gs.hand_label(h_idx);
        fb.text(
            layout.player_card_pos(h_idx, 0).x,
            layout.player_origin.y + CARD_H + 1,
            &label,
            if active { Color::Yellow } else { Color::White },
            table_bg,
        );

        fb.text(
            layout.player_card_pos(h_idx, 0).x,
            layout.player_origin.y + CARD_H + 2,
            &format!("Bet: {}", gs.wagers[h_idx]),
            Color::White,
            table_bg,
        );
    }

    let left_x = layout.table.x + 2;
    let split_x = layout.table.x + layout.table.w / 2;
    let right_x = split_x + 2;
    let panel_bg = Color::Black;
    for y in (layout.bar_y - 1)..=(layout.bar_y + 1) {
        fb.hline(layout.table.x + 1, y, layout.table.w - 2, ' ', Color::White, panel_bg);
    }
    for y in (layout.bar_y - 1)..=(layout.bar_y + 1) {
        fb.set(split_x, y, '│', Color::DarkGrey, panel_bg);
    }

    fb.text(left_x, layout.bar_y - 1, "CHIPS", Color::Yellow, panel_bg);
    fb.text(
        left_x + 6,
        layout.bar_y - 1,
        &format!("{}", gs.chips),
        Color::White,
        panel_bg,
    );
    fb.text(left_x + 12, layout.bar_y - 1, "BET", Color::Yellow, panel_bg);
    fb.text(
        left_x + 16,
        layout.bar_y - 1,
        &format!("{}", gs.bet),
        Color::White,
        panel_bg,
    );
    let phase_color = match gs.phase {
        Phase::Betting => Color::Cyan,
        Phase::PlayerTurn => Color::Yellow,
        Phase::DealerTurn => Color::Magenta,
        Phase::Payout => Color::Green,
        Phase::Shuffle => Color::Blue,
        Phase::Dealing => Color::White,
    };
    fb.text(left_x, layout.bar_y, "PHASE", Color::Yellow, panel_bg);
    fb.text(
        left_x + 6,
        layout.bar_y,
        &format!("{:?}", gs.phase),
        phase_color,
        panel_bg,
    );
    let msg_max = (split_x - left_x - 2).max(0) as usize;
    let short_msg: String = gs.message.chars().take(msg_max).collect();
    fb.text(left_x, layout.bar_y + 1, &short_msg, Color::White, panel_bg);

    fb.text(right_x, layout.bar_y - 1, "CONTROLS", Color::Yellow, panel_bg);
    if gs.phase == Phase::Betting {
        fb.text(right_x, layout.bar_y, "BET", Color::Cyan, panel_bg);
        fb.text(right_x + 4, layout.bar_y, "<- -> +/-", Color::Green, panel_bg);
        fb.text(right_x + 15, layout.bar_y, "DEAL", Color::Cyan, panel_bg);
        fb.text(right_x + 20, layout.bar_y, "Enter", Color::Green, panel_bg);
        fb.text(right_x, layout.bar_y + 1, "MENU", Color::Cyan, panel_bg);
        fb.text(
            right_x + 5,
            layout.bar_y + 1,
            "r reset   q quit",
            Color::White,
            panel_bg,
        );
    } else {
        fb.text(right_x, layout.bar_y, "PLAY", Color::Cyan, panel_bg);
        fb.text(
            right_x + 5,
            layout.bar_y,
            "h hit  s stand  d double",
            Color::Green,
            panel_bg,
        );
        fb.text(right_x, layout.bar_y + 1, "SPLIT", Color::Cyan, panel_bg);
        fb.text(
            right_x + 6,
            layout.bar_y + 1,
            "p   r reset   q quit",
            Color::White,
            panel_bg,
        );
    }

    render_anim_overlays(fb, gs, table_bg);
}

fn render_anim_overlays(fb: &mut FrameBuffer, gs: &GameState, bg: Color) {
    if let Some(active) = &gs.anim.active {
        match &active.event {
            AnimEvent::Deal {
                card,
                from,
                to,
                duration,
                ..
            } => {
                let t = (active.elapsed / duration).clamp(0.0, 1.0);
                let pos = Vec2 {
                    x: lerp(from.x, to.x, t),
                    y: lerp(from.y, to.y, t),
                };
                draw_card_sprite(fb, pos, Some(*card), false, false, None, bg);
            }
            AnimEvent::FlipHole { at, duration } => {
                let t = (active.elapsed / duration).clamp(0.0, 1.0);
                let edge = (2.0 * t - 1.0).abs();
                let thin = max(1, (CARD_W as f32 * edge).round() as i32);
                let face_down = t < 0.5;
                draw_card_sprite(fb, *at, None, face_down, false, Some(thin), bg);
            }
            AnimEvent::Flash { rect, duration } => {
                let t = (active.elapsed / duration).clamp(0.0, 1.0);
                let pulse = ((t * 8.0).floor() as i32) % 2 == 0;
                if pulse {
                    fb.rect_border(*rect, Color::Yellow, bg);
                }
            }
            AnimEvent::ChipDelta {
                amount,
                at,
                duration,
            } => {
                let t = (active.elapsed / duration).clamp(0.0, 1.0);
                let y = at.y - (t * 2.0) as i32;
                let txt = if *amount > 0 {
                    format!("+{}", amount)
                } else if *amount < 0 {
                    format!("{}", amount)
                } else {
                    "PUSH".to_string()
                };
                let col = if *amount > 0 {
                    Color::Green
                } else if *amount < 0 {
                    Color::Red
                } else {
                    Color::White
                };
                fb.text(at.x, y, &txt, col, bg);
            }
        }
    }
}

fn flush_diff(stdout: &mut Stdout, front: &FrameBuffer, back: &FrameBuffer) -> Result<()> {
    queue!(stdout, BeginSynchronizedUpdate)?;
    for y in 0..back.h {
        for x in 0..back.w {
            let idx = y as usize * back.w as usize + x as usize;
            if front.cells[idx] != back.cells[idx] {
                let c = back.cells[idx];
                queue!(
                    stdout,
                    MoveTo(x, y),
                    SetForegroundColor(c.fg),
                    SetBackgroundColor(c.bg),
                    Print(c.ch)
                )?;
            }
        }
    }
    queue!(stdout, EndSynchronizedUpdate)?;
    stdout.flush()?;
    Ok(())
}

fn apply_event_commit(gs: &mut GameState, event: AnimEvent) {
    match event {
        AnimEvent::Deal { card, dest, .. } => match dest {
            DealDest::Player { hand } => {
                if let Some(h) = gs.players.get_mut(hand) {
                    h.cards.push(card);
                    let (tot, _) = h.totals();
                    h.busted = tot > 21;
                }
            }
            DealDest::DealerUp | DealDest::DealerDown => {
                gs.dealer.cards.push(card);
            }
        },
        AnimEvent::FlipHole { .. } => {
            gs.dealer_reveal = DealerReveal::Revealed;
        }
        AnimEvent::Flash { .. } | AnimEvent::ChipDelta { .. } => {}
    }
}

fn advance_animations(gs: &mut GameState, dt: f32) {
    gs.anim.now += dt;

    if gs.anim.active.is_none() {
        if let Some(next) = gs.anim.queue.pop_front() {
            gs.anim.active = Some(ActiveAnim {
                event: next,
                elapsed: 0.0,
            });
        }
    }

    if let Some(active) = &mut gs.anim.active {
        active.elapsed += dt;
        let duration = match &active.event {
            AnimEvent::Deal { duration, .. }
            | AnimEvent::FlipHole { duration, .. }
            | AnimEvent::Flash { duration, .. }
            | AnimEvent::ChipDelta { duration, .. } => *duration,
        };
        if active.elapsed >= duration {
            let done = gs.anim.active.take().expect("active anim missing");
            apply_event_commit(gs, done.event);
            gs.post_deal_checks();
        }
    }
}

fn parse_seed() -> u64 {
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--seed" {
            if let Some(val) = args.next() {
                if let Ok(seed) = val.parse::<u64>() {
                    return seed;
                }
            }
        }
    }
    1
}

fn adjust_bet(gs: &mut GameState, delta: i64) {
    let max_bet = gs.chips.max(MIN_BET);
    gs.bet = (gs.bet + delta).clamp(MIN_BET, max_bet);
}

fn handle_key(gs: &mut GameState, key: KeyEvent, layout: Layout, seed: u64, running: &mut bool) {
    if key.kind != KeyEventKind::Press {
        return;
    }

    match key.code {
        KeyCode::Char('q') => {
            *running = false;
            return;
        }
        KeyCode::Char('r') => {
            gs.reset_all(seed);
            return;
        }
        _ => {}
    }

    if gs.phase == Phase::Betting {
        match key.code {
            KeyCode::Left | KeyCode::Char('-') => adjust_bet(gs, -BET_STEP),
            KeyCode::Right | KeyCode::Char('+') => adjust_bet(gs, BET_STEP),
            KeyCode::Enter => gs.start_round(layout),
            _ => {}
        }
        return;
    }

    if gs.anim.is_busy() {
        return;
    }

    if gs.phase == Phase::PlayerTurn {
        if let KeyCode::Char(ch) = key.code {
            gs.handle_player_action(ch.to_ascii_lowercase(), layout);
        }
    }
}

struct TerminalGuard;

impl TerminalGuard {
    fn setup() -> Result<Self> {
        enable_raw_mode().context("enable raw mode")?;
        execute!(
            stdout(),
            EnterAlternateScreen,
            Hide,
            DisableLineWrap,
            SetBackgroundColor(Color::Black),
            SetForegroundColor(Color::White)
        )
        .context("terminal enter alt screen")?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(
            stdout(),
            EndSynchronizedUpdate,
            EnableLineWrap,
            Show,
            LeaveAlternateScreen,
            SetBackgroundColor(Color::Black),
            SetForegroundColor(Color::White)
        );
        let _ = disable_raw_mode();
    }
}

fn run(seed: u64) -> Result<()> {
    let _guard = TerminalGuard::setup()?;
    let mut gs = GameState::new(seed);

    let (mut w, mut h) = terminal::size().context("terminal size")?;
    let mut front = FrameBuffer::new(w, h);
    front.clear(Color::Black);
    let mut back = FrameBuffer::new(w, h);

    let mut last = Instant::now();
    let frame_time = Duration::from_secs_f32(1.0 / FPS_CAP);
    let mut running = true;

    while running {
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(key) => {
                    let layout = Layout::compute(w, h);
                    handle_key(&mut gs, key, layout, seed, &mut running);
                }
                Event::Resize(nw, nh) => {
                    w = nw;
                    h = nh;
                    front = FrameBuffer::new(w, h);
                    back = FrameBuffer::new(w, h);
                }
                _ => {}
            }
        }

        let now = Instant::now();
        let mut dt = (now - last).as_secs_f32();
        last = now;
        dt = dt.min(DT_CLAMP);

        advance_animations(&mut gs, dt);

        let layout = Layout::compute(w, h);

        if gs.phase == Phase::Dealing && !gs.anim.is_busy() {
            gs.on_dealing_finished(layout);
        }

        if gs.phase == Phase::DealerTurn {
            if matches!(gs.dealer_reveal, DealerReveal::Hidden) && !gs.anim.is_busy() {
                gs.start_dealer_turn(layout);
            }
            gs.update_dealer_policy(layout);
        }

        if gs.phase == Phase::Payout {
            if gs.payout_timer > 0.0 {
                gs.payout_timer -= dt;
            }
            if gs.payout_timer <= 0.0 && !gs.anim.is_busy() {
                gs.finish_round();
            }
        }

        if gs.phase == Phase::Shuffle {
            if gs.payout_timer > 0.0 {
                gs.payout_timer -= dt;
            }
            if gs.payout_timer <= 0.0 {
                gs.shoe = Shoe::new(gs.shoe.decks, &mut gs.rng);
                gs.phase = Phase::Betting;
                gs.message = "Shoe shuffled. Place bet.".to_string();
            }
        }

        if gs.phase == Phase::PlayerTurn {
            gs.progress_active_hand();
        }

        render_game(&mut back, &gs, layout);
        flush_diff(&mut stdout(), &front, &back)?;
        std::mem::swap(&mut front, &mut back);

        let elapsed = last.elapsed();
        if elapsed < frame_time {
            std::thread::sleep(frame_time - elapsed);
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    let seed = parse_seed();
    run(seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_first_20_cards() {
        let mut rng1 = ChaCha8Rng::seed_from_u64(123);
        let mut rng2 = ChaCha8Rng::seed_from_u64(123);
        let mut s1 = Shoe::new(NUM_DECKS, &mut rng1);
        let mut s2 = Shoe::new(NUM_DECKS, &mut rng2);
        let a: Vec<Card> = (0..20).map(|_| s1.draw()).collect();
        let b: Vec<Card> = (0..20).map(|_| s2.draw()).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn blackjack_detection() {
        let mut hand = Hand::new();
        hand.cards.push(Card {
            rank: Rank::A,
            suit: Suit::Spades,
        });
        hand.cards.push(Card {
            rank: Rank::K,
            suit: Suit::Hearts,
        });
        assert!(hand.is_blackjack());
    }

    #[test]
    fn soft_total() {
        let mut hand = Hand::new();
        hand.cards.push(Card {
            rank: Rank::A,
            suit: Suit::Spades,
        });
        hand.cards.push(Card {
            rank: Rank::Six,
            suit: Suit::Hearts,
        });
        assert_eq!(hand.totals(), (17, true));
    }
}
