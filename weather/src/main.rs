use std::{
    collections::HashMap,
    io::{self, Stdout},
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Local, NaiveDate};
use clap::Parser;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{
        self, BeginSynchronizedUpdate, DisableLineWrap, EnableLineWrap, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use image::ImageFormat;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::*,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::*,
};
use serde::Deserialize;
use tokio::sync::{mpsc, RwLock};

#[derive(Parser, Debug, Clone)]
#[command(name = "weather")]
#[command(about = "Terminal weather (Open-Meteo + RainViewer radar)")]
struct Cli {
    /// Latitude (decimal). Example: 44.31
    #[arg(long, allow_negative_numbers = true)]
    lat: Option<f64>,

    /// Longitude (decimal). Example: -69.78
    #[arg(long, allow_negative_numbers = true)]
    lon: Option<f64>,

    /// ZIP / Postal code (uses Zippopotam.us). If provided, overrides lat/lon.
    #[arg(long)]
    zip: Option<String>,

    /// Country code for --zip (default: us)
    #[arg(long, default_value = "us")]
    country: String,

    /// Refresh interval for forecast (minutes)
    #[arg(long, default_value_t = 15)]
    forecast_refresh_min: u64,

    /// Refresh interval for radar (minutes)
    #[arg(long, default_value_t = 5)]
    radar_refresh_min: u64,

    /// Radar zoom (free RainViewer is limited; keep <= 10)
    #[arg(long, default_value_t = 6)]
    radar_zoom: u8,

    /// Force monochrome (no colors)
    #[arg(long, default_value_t = false)]
    mono: bool,
}

#[derive(Debug, Clone)]
struct Location {
    name: String,
    lat: f64,
    lon: f64,
}

#[derive(Debug, Clone)]
struct ForecastData {
    fetched_at: DateTime<Local>,
    current: CurrentSummary,
    hourly: Vec<HourRow>,
    daily: Vec<DayRow>,
}

#[derive(Debug, Clone)]
struct RadarData {
    fetched_at: DateTime<Local>,
    frame_time_utc: i64,
    zoom: u8,
    cells: Vec<Vec<RadarCell>>,
    info: String,
}

#[derive(Debug, Clone)]
struct CurrentSummary {
    temp_c: f64,
    wind_kph: f64,
    cloud_pct: f64,
    precip_mm: f64,
    precip_prob_pct: f64,
    code: i32,
    time_local: String,
}

#[derive(Debug, Clone)]
struct HourRow {
    time_local: String,
    temp_c: f64,
    precip_mm: f64,
    precip_prob_pct: f64,
    wind_kph: f64,
    cloud_pct: f64,
    code: i32,
}

#[derive(Debug, Clone)]
struct DayRow {
    date: NaiveDate,
    tmax_c: f64,
    tmin_c: f64,
    precip_prob_max_pct: f64,
    code: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Now,
    Hourly,
    Daily,
    Radar,
    Help,
}

impl Tab {
    fn all() -> &'static [Tab] {
        &[Tab::Now, Tab::Hourly, Tab::Daily, Tab::Radar, Tab::Help]
    }
    fn title(self) -> &'static str {
        match self {
            Tab::Now => "Now",
            Tab::Hourly => "Hourly",
            Tab::Daily => "Daily",
            Tab::Radar => "Radar",
            Tab::Help => "Help",
        }
    }
}

#[derive(Debug)]
struct AppState {
    location: Location,
    mono: bool,
    tab: Tab,
    temp_unit: TempUnit,
    forecast: Option<ForecastData>,
    radar: Option<RadarData>,
    last_error: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let loc = resolve_location(&cli).await?;
    let mut state = AppState {
        location: loc,
        mono: cli.mono,
        tab: Tab::Now,
        temp_unit: TempUnit::C,
        forecast: None,
        radar: None,
        last_error: None,
    };

    // Initial fetch (best-effort; UI still starts if one fails)
    match fetch_forecast(state.location.lat, state.location.lon).await {
        Ok(fc) => state.forecast = Some(fc),
        Err(e) => state.last_error = Some(format!("forecast: {e:#}")),
    }
    match fetch_radar(state.location.lat, state.location.lon, cli.radar_zoom).await {
        Ok(rd) => state.radar = Some(rd),
        Err(e) => state.last_error = Some(format!("radar: {e:#}")),
    }

    // Start background refresh tasks
    let shared = RwLock::new(state);

    let (tx, mut rx) = mpsc::channel::<Cmd>(16);

    spawn_forecast_refresher(
        tx.clone(),
        Duration::from_secs(cli.forecast_refresh_min * 60),
    );
    spawn_radar_refresher(tx.clone(), Duration::from_secs(cli.radar_refresh_min * 60));

    // TUI setup
    let mut terminal = setup_terminal()?;
    let mut last_tick = Instant::now();

    loop {
        // Drain commands coming from background refreshers and manual refreshes
        while let Ok(cmd) = rx.try_recv() {
            handle_cmd(&shared, cmd).await;
        }

        // Render
        let snapshot = { shared.read().await.clone_for_render() };
        draw_frame(&mut terminal, &snapshot)?;

        // Input
        let timeout = Duration::from_millis(33);
        if event::poll(timeout)? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    if handle_key(&tx, &shared, &snapshot, k.code).await? {
                        break;
                    }
                }
            }
        }

        // Soft tick to keep UI responsive
        if last_tick.elapsed() >= Duration::from_millis(250) {
            last_tick = Instant::now();
        }
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}

#[derive(Debug)]
enum Cmd {
    RefreshForecast,
    RefreshRadar,
    SetTab(Tab),
    AdjustRadarZoom(i8),
    ClearError,
    ToggleTempUnit,
}

impl AppState {
    fn clone_for_render(&self) -> RenderState {
        RenderState {
            location: self.location.clone(),
            mono: self.mono,
            tab: self.tab,
            temp_unit: self.temp_unit,
            forecast: self.forecast.clone(),
            radar: self.radar.clone(),
            last_error: self.last_error.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TempUnit {
    C,
    F,
}

impl TempUnit {
    fn toggle(self) -> Self {
        match self {
            TempUnit::C => TempUnit::F,
            TempUnit::F => TempUnit::C,
        }
    }
}

#[derive(Debug, Clone)]
struct RenderState {
    location: Location,
    mono: bool,
    tab: Tab,
    temp_unit: TempUnit,
    forecast: Option<ForecastData>,
    radar: Option<RadarData>,
    last_error: Option<String>,
}

async fn handle_key(
    tx: &mpsc::Sender<Cmd>,
    shared: &RwLock<AppState>,
    snap: &RenderState,
    code: KeyCode,
) -> Result<bool> {
    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(true),
        KeyCode::Left => {
            let tabs = Tab::all();
            let i = tabs.iter().position(|t| *t == snap.tab).unwrap_or(0);
            let ni = if i == 0 { tabs.len() - 1 } else { i - 1 };
            tx.send(Cmd::SetTab(tabs[ni])).await.ok();
        }
        KeyCode::Right => {
            let tabs = Tab::all();
            let i = tabs.iter().position(|t| *t == snap.tab).unwrap_or(0);
            let ni = (i + 1) % tabs.len();
            tx.send(Cmd::SetTab(tabs[ni])).await.ok();
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            tx.send(Cmd::RefreshForecast).await.ok();
            tx.send(Cmd::RefreshRadar).await.ok();
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            tx.send(Cmd::AdjustRadarZoom(1)).await.ok();
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            tx.send(Cmd::AdjustRadarZoom(-1)).await.ok();
        }
        KeyCode::Char('c') | KeyCode::Char('C') => {
            // quick clear error if any
            tx.send(Cmd::ClearError).await.ok();
        }
        KeyCode::Char('f') | KeyCode::Char('F') => {
            tx.send(Cmd::ToggleTempUnit).await.ok();
        }
        _ => {}
    }

    // Also directly update tab in shared state quickly (so it feels instant)
    match code {
        KeyCode::Left | KeyCode::Right => {
            // command already sent; nothing else
            let _ = shared;
        }
        _ => {}
    }

    Ok(false)
}

async fn handle_cmd(shared: &RwLock<AppState>, cmd: Cmd) {
    match cmd {
        Cmd::SetTab(t) => {
            let mut st = shared.write().await;
            st.tab = t;
        }
        Cmd::ClearError => {
            let mut st = shared.write().await;
            st.last_error = None;
        }
        Cmd::ToggleTempUnit => {
            let mut st = shared.write().await;
            st.temp_unit = st.temp_unit.toggle();
        }
        Cmd::AdjustRadarZoom(delta) => {
            let (lat, lon, new_zoom) = {
                let mut st = shared.write().await;
                let z = st
                    .radar
                    .as_ref()
                    .map(|r| r.zoom)
                    .unwrap_or(6)
                    .clamp(1, 10);
                let nz = (z as i16 + delta as i16).clamp(1, 10) as u8;
                st.last_error = None;
                (st.location.lat, st.location.lon, nz)
            };

            match fetch_radar(lat, lon, new_zoom).await {
                Ok(rd) => {
                    let mut st = shared.write().await;
                    st.radar = Some(rd);
                }
                Err(e) => {
                    let mut st = shared.write().await;
                    st.last_error = Some(format!("radar: {e:#}"));
                }
            }
        }
        Cmd::RefreshForecast => {
            let (lat, lon) = {
                let st = shared.read().await;
                (st.location.lat, st.location.lon)
            };
            match fetch_forecast(lat, lon).await {
                Ok(fc) => {
                    let mut st = shared.write().await;
                    st.forecast = Some(fc);
                    st.last_error = None;
                }
                Err(e) => {
                    let mut st = shared.write().await;
                    st.last_error = Some(format!("forecast: {e:#}"));
                }
            }
        }
        Cmd::RefreshRadar => {
            let (lat, lon, zoom) = {
                let st = shared.read().await;
                let z = st.radar.as_ref().map(|r| r.zoom).unwrap_or(6);
                (st.location.lat, st.location.lon, z)
            };
            match fetch_radar(lat, lon, zoom).await {
                Ok(rd) => {
                    let mut st = shared.write().await;
                    st.radar = Some(rd);
                    st.last_error = None;
                }
                Err(e) => {
                    let mut st = shared.write().await;
                    st.last_error = Some(format!("radar: {e:#}"));
                }
            }
        }
    }
}

fn spawn_forecast_refresher(tx: mpsc::Sender<Cmd>, every: Duration) {
    tokio::spawn(async move {
        let mut t = tokio::time::interval(every);
        loop {
            t.tick().await;
            tx.send(Cmd::RefreshForecast).await.ok();
        }
    });
}

fn spawn_radar_refresher(tx: mpsc::Sender<Cmd>, every: Duration) {
    tokio::spawn(async move {
        let mut t = tokio::time::interval(every);
        loop {
            t.tick().await;
            tx.send(Cmd::RefreshRadar).await.ok();
        }
    });
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    terminal::enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(
        out,
        EnterAlternateScreen,
        DisableLineWrap,
        cursor::Hide
    )?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    Ok(terminal)
}

fn restore_terminal(term: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let mut out = io::stdout();
    execute!(
        out,
        BeginSynchronizedUpdate,
        cursor::Show,
        EnableLineWrap,
        LeaveAlternateScreen,
        EndSynchronizedUpdate
    )?;
    terminal::disable_raw_mode()?;
    term.show_cursor()?;
    Ok(())
}

fn draw_frame(term: &mut Terminal<CrosstermBackend<Stdout>>, st: &RenderState) -> Result<()> {
    let mono = st.mono;

    term.draw(|f| {
        // Synchronized update reduces tearing over SSH/Windows Terminal in practice.
        let _ = execute!(io::stdout(), BeginSynchronizedUpdate);

        let area = f.size();
        let outer = Block::default()
            .borders(Borders::ALL)
            .title(Line::from(vec![
                Span::styled(" weather ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" "),
                Span::styled(
                    format!("{} ({:.4}, {:.4})", st.location.name, st.location.lat, st.location.lon),
                    Style::default().fg(if mono { Color::White } else { Color::Cyan }),
                ),
            ]))
            .border_style(Style::default().fg(if mono { Color::Gray } else { Color::DarkGray }));
        f.render_widget(outer, area);

        let inner = area.inner(Margin { horizontal: 1, vertical: 1 });
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(3)])
            .split(inner);

        render_tabs(f, rows[0], st);
        render_main(f, rows[1], st);
        render_footer(f, rows[2], st);

        let _ = execute!(io::stdout(), EndSynchronizedUpdate);
    })?;

    Ok(())
}

fn render_tabs(f: &mut Frame, area: Rect, st: &RenderState) {
    let titles: Vec<Line> = Tab::all()
        .iter()
        .map(|t| Line::from(Span::raw(t.title())))
        .collect();

    let idx = Tab::all()
        .iter()
        .position(|t| *t == st.tab)
        .unwrap_or(0);

    let tabs = Tabs::new(titles)
        .select(idx)
        .block(Block::default().borders(Borders::ALL).title("View"))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .divider(" | ");
    f.render_widget(tabs, area);
}

fn render_footer(f: &mut Frame, area: Rect, st: &RenderState) {
    let mut spans = vec![
        Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" quit  "),
        Span::styled("←/→", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" tabs  "),
        Span::styled("r", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" refresh  "),
        Span::styled("+/-", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" radar zoom  "),
        Span::styled("f", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" C/F  "),
        Span::styled("c", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" clear error"),
    ];

    if let Some(e) = &st.last_error {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(
            format!("ERR: {e}"),
            Style::default().fg(if st.mono { Color::White } else { Color::Red }),
        ));
    }

    let p = Paragraph::new(Line::from(spans))
        .block(Block::default().borders(Borders::ALL).title("Keys"));
    f.render_widget(p, area);
}

fn render_main(f: &mut Frame, area: Rect, st: &RenderState) {
    match st.tab {
        Tab::Now => render_now(f, area, st),
        Tab::Hourly => render_hourly(f, area, st),
        Tab::Daily => render_daily(f, area, st),
        Tab::Radar => render_radar(f, area, st),
        Tab::Help => render_help(f, area, st),
    }
}

fn render_now(f: &mut Frame, area: Rect, st: &RenderState) {
    let mono = st.mono;
    let block = Block::default().borders(Borders::ALL).title("Current");
    if let Some(fc) = &st.forecast {
        let icon = code_icon(fc.current.code);
        let desc = code_desc(fc.current.code);
        let (temp, unit) = format_temp(fc.current.temp_c, st.temp_unit);
        let lines = vec![
            Line::from(vec![
                Span::styled(icon, Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" "),
                Span::styled(desc, Style::default().add_modifier(Modifier::BOLD)),
                Span::raw("   "),
                Span::raw(fc.current.time_local.clone()),
            ]),
            Line::from(format!(
                "Temp: {:.1}°{}   Wind: {:.0} km/h   Cloud: {:.0}%   Precip: {:.1} mm ({}%)",
                temp,
                unit,
                fc.current.wind_kph,
                fc.current.cloud_pct,
                fc.current.precip_mm,
                fc.current.precip_prob_pct.round()
            )),
            Line::from(format!("Fetched: {}", fc.fetched_at.format("%Y-%m-%d %H:%M:%S"))),
        ];

        f.render_widget(
            Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: true }),
            area,
        );
    } else {
        f.render_widget(
            Paragraph::new("No forecast loaded yet (press r).")
                .style(Style::default().fg(if mono { Color::White } else { Color::Yellow }))
                .block(block),
            area,
        );
    }
}

fn render_hourly(f: &mut Frame, area: Rect, st: &RenderState) {
    let mono = st.mono;
    let block = Block::default().borders(Borders::ALL).title("Hourly (next ~24h)");
    if let Some(fc) = &st.forecast {
        let rows: Vec<Row> = fc
            .hourly
            .iter()
            .take(24)
            .map(|h| {
                let icon = code_icon(h.code);
                let (temp, unit) = format_temp(h.temp_c, st.temp_unit);
                Row::new(vec![
                    Cell::from(h.time_local.clone()),
                    Cell::from(icon),
                    Cell::from(format!("{:.1}°{}", temp, unit)),
                    Cell::from(format!("{:.0}%", h.cloud_pct)),
                    Cell::from(format!("{:.0}%", h.precip_prob_pct)),
                    Cell::from(format!("{:.1}mm", h.precip_mm)),
                    Cell::from(format!("{:.0}km/h", h.wind_kph)),
                ])
            })
            .collect();

        let header = Row::new(vec![
            "Time", "", "Temp", "Cloud", "PoP", "Precip", "Wind",
        ])
        .style(Style::default().add_modifier(Modifier::BOLD));

        let t = Table::new(
            rows,
            [
                Constraint::Length(17),
                Constraint::Length(2),
                Constraint::Length(8),
                Constraint::Length(7),
                Constraint::Length(6),
                Constraint::Length(9),
                Constraint::Length(9),
            ],
        )
        .header(header)
        .block(block)
        .column_spacing(1);

        f.render_widget(t, area);
    } else {
        f.render_widget(
            Paragraph::new("No forecast loaded yet (press r).")
                .style(Style::default().fg(if mono { Color::White } else { Color::Yellow }))
                .block(block),
            area,
        );
    }
}

fn render_daily(f: &mut Frame, area: Rect, st: &RenderState) {
    let mono = st.mono;
    let block = Block::default().borders(Borders::ALL).title("Daily (10 days)");
    if let Some(fc) = &st.forecast {
        let rows: Vec<Row> = fc
            .daily
            .iter()
            .take(10)
            .map(|d| {
                let icon = code_icon(d.code);
                let (tmax, unit) = format_temp(d.tmax_c, st.temp_unit);
                let (tmin, _) = format_temp(d.tmin_c, st.temp_unit);
                Row::new(vec![
                    Cell::from(d.date.format("%a %Y-%m-%d").to_string()),
                    Cell::from(icon),
                    Cell::from(format!("{:.1}°{}", tmax, unit)),
                    Cell::from(format!("{:.1}°{}", tmin, unit)),
                    Cell::from(format!("{:.0}%", d.precip_prob_max_pct)),
                    Cell::from(code_desc(d.code)),
                ])
            })
            .collect();

        let header = Row::new(vec![
            "Date", "", "Max", "Min", "PoP", "Summary",
        ])
        .style(Style::default().add_modifier(Modifier::BOLD));

        let t = Table::new(
            rows,
            [
                Constraint::Length(14),
                Constraint::Length(2),
                Constraint::Length(7),
                Constraint::Length(7),
                Constraint::Length(6),
                Constraint::Min(10),
            ],
        )
        .header(header)
        .block(block)
        .column_spacing(1);

        f.render_widget(t, area);
    } else {
        f.render_widget(
            Paragraph::new("No forecast loaded yet (press r).")
                .style(Style::default().fg(if mono { Color::White } else { Color::Yellow }))
                .block(block),
            area,
        );
    }
}

fn render_radar(f: &mut Frame, area: Rect, st: &RenderState) {
    let mono = st.mono;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    let block = Block::default().borders(Borders::ALL).title("Radar (Braille)");
    if let Some(rd) = &st.radar {
        let inner = block.inner(chunks[0]);
        let target_w = inner.width as usize;
        let target_h = inner.height as usize;
        let lines = if target_w == 0 || target_h == 0 {
            Vec::new()
        } else {
            let fitted = fit_radar_cells(&rd.cells, target_w, target_h);
            radar_lines(&fitted, mono)
        };
        let p = Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false });
        f.render_widget(p, chunks[0]);

        let info = format!(
            "{} | frame UTC: {} | zoom: {} | fetched: {}",
            rd.info,
            rd.frame_time_utc,
            rd.zoom,
            rd.fetched_at.format("%Y-%m-%d %H:%M:%S")
        );
        f.render_widget(
            Paragraph::new(info)
                .block(Block::default().borders(Borders::ALL).title("Info"))
                .wrap(Wrap { trim: true }),
            chunks[1],
        );
    } else {
        f.render_widget(
            Paragraph::new("No radar loaded yet (press r).")
                .style(Style::default().fg(if mono { Color::White } else { Color::Yellow }))
                .block(block),
            chunks[0],
        );
        f.render_widget(
            Paragraph::new(" ")
                .block(Block::default().borders(Borders::ALL).title("Info")),
            chunks[1],
        );
    }
}

fn radar_lines(cells: &[Vec<RadarCell>], mono: bool) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(cells.len());
    for row in cells {
        if row.is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut current_ink = row[0].ink;
        let mut buf = String::new();
        for cell in row {
            if cell.ink != current_ink {
                let chunk = std::mem::take(&mut buf);
                spans.push(Span::styled(chunk, radar_style(current_ink, mono)));
                current_ink = cell.ink;
            }
            buf.push(cell.ch);
        }
        if !buf.is_empty() {
            spans.push(Span::styled(buf, radar_style(current_ink, mono)));
        }
        lines.push(Line::from(spans));
    }
    lines
}

fn fit_radar_cells(cells: &[Vec<RadarCell>], target_w: usize, target_h: usize) -> Vec<Vec<RadarCell>> {
    if cells.is_empty() || target_w == 0 || target_h == 0 {
        return Vec::new();
    }
    let src_h = cells.len();
    let src_w = cells[0].len().max(1);
    let scale_w = target_w as f32 / src_w as f32;
    let scale_h = target_h as f32 / src_h as f32;
    let scale = scale_w.min(scale_h);

    let (fit_w, fit_h) = if scale >= 1.0 {
        (src_w, src_h)
    } else {
        (
            (src_w as f32 * scale).floor().max(1.0) as usize,
            (src_h as f32 * scale).floor().max(1.0) as usize,
        )
    };

    let scaled = if fit_w == src_w && fit_h == src_h {
        cells.to_vec()
    } else {
        let mut out = Vec::with_capacity(fit_h);
        for y in 0..fit_h {
            let sy = (y * src_h) / fit_h;
            let row = &cells[sy];
            let mut out_row = Vec::with_capacity(fit_w);
            for x in 0..fit_w {
                let sx = (x * src_w) / fit_w;
                out_row.push(row[sx]);
            }
            out.push(out_row);
        }
        out
    };

    pad_radar_cells(&scaled, target_w, target_h)
}

fn pad_radar_cells(cells: &[Vec<RadarCell>], target_w: usize, target_h: usize) -> Vec<Vec<RadarCell>> {
    let blank = RadarCell { ch: ' ', ink: RadarInk::None };
    let src_h = cells.len();
    let src_w = cells.get(0).map(|row| row.len()).unwrap_or(0);
    let mut out = Vec::with_capacity(target_h);
    let pad_x = target_w.saturating_sub(src_w) / 2;
    let pad_y = target_h.saturating_sub(src_h) / 2;

    for y in 0..target_h {
        let mut row = Vec::with_capacity(target_w);
        if y < pad_y || y >= pad_y + src_h {
            row.resize(target_w, blank);
        } else {
            let src_row = &cells[y - pad_y];
            row.extend(std::iter::repeat(blank).take(pad_x));
            row.extend_from_slice(src_row);
            row.resize(target_w, blank);
        }
        out.push(row);
    }
    out
}

fn radar_style(ink: RadarInk, mono: bool) -> Style {
    match ink {
        RadarInk::Radar => Style::default().fg(if mono { Color::White } else { Color::Cyan }),
        RadarInk::Border => Style::default().fg(if mono { Color::Gray } else { Color::White }),
        RadarInk::None => Style::default(),
    }
}

fn render_help(f: &mut Frame, area: Rect, st: &RenderState) {
    let mono = st.mono;
    let lines = vec![
        Line::from("Sources:"),
        Line::from("  • Forecast: Open-Meteo (10-day + hourly, no API key)"),
        Line::from("  • Radar: RainViewer tiles (past frames)"),
        Line::from(""),
        Line::from("CLI examples:"),
        Line::from("  weather --lat 44.31 --lon -69.78"),
        Line::from("  weather --zip 04901"),
        Line::from("  weather --zip 04901 --country us --radar-zoom 7"),
        Line::from(""),
        Line::from("Keys: q quit | ←/→ tabs | r refresh | +/- radar zoom | f C/F | c clear error"),
        Line::from(""),
        Line::from("Notes:"),
        Line::from("  • Radar nowcast availability changes over time; this uses 'past' frames."),
        Line::from("  • If radar is blank, try a lower zoom (e.g., 5–7)."),
    ];

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .style(Style::default().fg(if mono { Color::White } else { Color::Gray }))
        .wrap(Wrap { trim: true });
    f.render_widget(p, area);
}

fn format_temp(c: f64, unit: TempUnit) -> (f64, &'static str) {
    match unit {
        TempUnit::C => (c, "C"),
        TempUnit::F => (c * 9.0 / 5.0 + 32.0, "F"),
    }
}

fn code_icon(code: i32) -> &'static str {
    match code {
        0 => "☀",
        1 | 2 => "⛅",
        3 => "☁",
        45 | 48 => "🌫",
        51 | 53 | 55 | 56 | 57 => "🌦",
        61 | 63 | 65 => "🌧",
        66 | 67 => "🌧",
        71 | 73 | 75 | 77 => "❄",
        80 | 81 | 82 => "🌧",
        85 | 86 => "❄",
        95 | 96 | 99 => "⛈",
        _ => "·",
    }
}

fn code_desc(code: i32) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Mostly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 => "Fog",
        48 => "Rime fog",
        51 => "Light drizzle",
        53 => "Drizzle",
        55 => "Dense drizzle",
        56 => "Freezing drizzle",
        57 => "Freezing drizzle",
        61 => "Light rain",
        63 => "Rain",
        65 => "Heavy rain",
        66 => "Freezing rain",
        67 => "Freezing rain",
        71 => "Light snow",
        73 => "Snow",
        75 => "Heavy snow",
        77 => "Snow grains",
        80 => "Rain showers",
        81 => "Rain showers",
        82 => "Violent showers",
        85 => "Snow showers",
        86 => "Heavy snow showers",
        95 => "Thunderstorm",
        96 => "T-storm hail",
        99 => "T-storm hail",
        _ => "Unknown",
    }
}

/* ----------------------------
   Location (lat/lon or ZIP)
---------------------------- */

async fn resolve_location(cli: &Cli) -> Result<Location> {
    if let Some(zip) = &cli.zip {
        let (lat, lon, name) = lookup_zip(&cli.country, zip).await?;
        return Ok(Location {
            name,
            lat,
            lon,
        });
    }

    let lat = cli.lat.ok_or_else(|| anyhow!("missing --lat (or use --zip)"))?;
    let lon = cli.lon.ok_or_else(|| anyhow!("missing --lon (or use --zip)"))?;

    Ok(Location {
        name: "custom coords".to_string(),
        lat,
        lon,
    })
}

#[derive(Debug, Deserialize)]
struct ZipResp {
    places: Vec<ZipPlace>,
}

#[derive(Debug, Deserialize)]
struct ZipPlace {
    #[serde(rename = "place name")]
    place_name: String,
    state: String,
    latitude: String,
    longitude: String,
}

async fn lookup_zip(country: &str, postal: &str) -> Result<(f64, f64, String)> {
    let url = format!("https://api.zippopotam.us/{}/{}", country, postal);
    let c = reqwest::Client::new();
    let resp = c
        .get(url)
        .send()
        .await
        .context("ZIP lookup request failed")?;

    if !resp.status().is_success() {
        return Err(anyhow!("ZIP lookup failed with HTTP {}", resp.status()));
    }

    let zr: ZipResp = resp.json().await.context("ZIP lookup JSON parse failed")?;
    let p = zr
        .places
        .get(0)
        .ok_or_else(|| anyhow!("ZIP lookup returned no places"))?;

    let lat: f64 = p.latitude.parse().context("ZIP latitude parse failed")?;
    let lon: f64 = p.longitude.parse().context("ZIP longitude parse failed")?;
    let name = format!("{}, {}", p.place_name, p.state);

    Ok((lat, lon, name))
}

/* ----------------------------
   Forecast (Open-Meteo)
---------------------------- */

#[derive(Debug, Deserialize)]
struct OpenMeteoResp {
    timezone: String,

    current: OpenMeteoCurrent,

    hourly: OpenMeteoHourly,
    daily: OpenMeteoDaily,
}

#[derive(Debug, Deserialize)]
struct OpenMeteoCurrent {
    time: String,
    temperature_2m: f64,
    wind_speed_10m: f64,
    cloud_cover: f64,
    precipitation: f64,
    precipitation_probability: f64,
    weather_code: i32,
}

#[derive(Debug, Deserialize)]
struct OpenMeteoHourly {
    time: Vec<String>,
    temperature_2m: Vec<f64>,
    precipitation: Vec<f64>,
    precipitation_probability: Vec<f64>,
    wind_speed_10m: Vec<f64>,
    cloud_cover: Vec<f64>,
    weather_code: Vec<i32>,
}

#[derive(Debug, Deserialize)]
struct OpenMeteoDaily {
    time: Vec<String>,
    temperature_2m_max: Vec<f64>,
    temperature_2m_min: Vec<f64>,
    precipitation_probability_max: Vec<f64>,
    weather_code: Vec<i32>,
}

async fn fetch_forecast(lat: f64, lon: f64) -> Result<ForecastData> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}\
&current=temperature_2m,wind_speed_10m,cloud_cover,precipitation,precipitation_probability,weather_code\
&hourly=temperature_2m,precipitation,precipitation_probability,wind_speed_10m,cloud_cover,weather_code\
&daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max,weather_code\
&forecast_days=10&timezone=auto"
    );

    let c = reqwest::Client::new();
    let resp = c
        .get(url)
        .send()
        .await
        .context("forecast request failed")?;

    if !resp.status().is_success() {
        return Err(anyhow!("forecast HTTP {}", resp.status()));
    }

    let om: OpenMeteoResp = resp.json().await.context("forecast JSON parse failed")?;

    let fetched_at = Local::now();

    let current = CurrentSummary {
        temp_c: om.current.temperature_2m,
        wind_kph: om.current.wind_speed_10m,
        cloud_pct: om.current.cloud_cover,
        precip_mm: om.current.precipitation,
        precip_prob_pct: om.current.precipitation_probability,
        code: om.current.weather_code,
        time_local: om.current.time.clone(),
    };

    let mut hourly = Vec::new();
    let n = om.hourly.time.len()
        .min(om.hourly.temperature_2m.len())
        .min(om.hourly.precipitation.len())
        .min(om.hourly.precipitation_probability.len())
        .min(om.hourly.wind_speed_10m.len())
        .min(om.hourly.cloud_cover.len())
        .min(om.hourly.weather_code.len());

    for i in 0..n {
        hourly.push(HourRow {
            time_local: om.hourly.time[i].clone(),
            temp_c: om.hourly.temperature_2m[i],
            precip_mm: om.hourly.precipitation[i],
            precip_prob_pct: om.hourly.precipitation_probability[i],
            wind_kph: om.hourly.wind_speed_10m[i],
            cloud_pct: om.hourly.cloud_cover[i],
            code: om.hourly.weather_code[i],
        });
    }

    let mut daily = Vec::new();
    let dn = om.daily.time.len()
        .min(om.daily.temperature_2m_max.len())
        .min(om.daily.temperature_2m_min.len())
        .min(om.daily.precipitation_probability_max.len())
        .min(om.daily.weather_code.len());

    for i in 0..dn {
        let date = NaiveDate::parse_from_str(&om.daily.time[i], "%Y-%m-%d")
            .unwrap_or_else(|_| NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());
        daily.push(DayRow {
            date,
            tmax_c: om.daily.temperature_2m_max[i],
            tmin_c: om.daily.temperature_2m_min[i],
            precip_prob_max_pct: om.daily.precipitation_probability_max[i],
            code: om.daily.weather_code[i],
        });
    }

    Ok(ForecastData {
        fetched_at,
        current,
        hourly,
        daily,
    })
}

/* ----------------------------
   Radar (RainViewer)
---------------------------- */

#[derive(Debug, Deserialize)]
struct RainViewerMaps {
    host: String,
    radar: Option<RainViewerRadar>,
}

#[derive(Debug, Deserialize)]
struct RainViewerRadar {
    past: Option<Vec<RainViewerFrame>>,
    nowcast: Option<Vec<RainViewerFrame>>,
}

#[derive(Debug, Deserialize)]
struct RainViewerFrame {
    time: i64,
    path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RadarInk {
    None,
    Border,
    Radar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RadarCell {
    ch: char,
    ink: RadarInk,
}

struct BasemapTiles {
    tiles: HashMap<(i32, i32), image::RgbaImage>,
    zoom: u8,
    top_left_world_x: f64,
    top_left_world_y: f64,
    scale_x: f64,
    scale_y: f64,
    sample_w: usize,
    sample_h: usize,
}

async fn fetch_radar(lat: f64, lon: f64, zoom: u8) -> Result<RadarData> {
    let zoom = zoom.clamp(1, 10);

    let c = reqwest::Client::new();
    let maps: RainViewerMaps = c
        .get("https://api.rainviewer.com/public/weather-maps.json")
        .send()
        .await
        .context("radar maps json request failed")?
        .error_for_status()
        .context("radar maps json HTTP error")?
        .json()
        .await
        .context("radar maps json parse failed")?;

    let radar = maps.radar.ok_or_else(|| anyhow!("radar section missing"))?;
    // Prefer latest past frame (most reliable over time)
    let frame = radar
        .past
        .as_ref()
        .and_then(|v| v.last())
        .or_else(|| radar.nowcast.as_ref().and_then(|v| v.first()))
        .ok_or_else(|| anyhow!("no radar frames available"))?;

    // Use the "lat/lon centered" URL form from RainViewer docs.
    // Format: {path}/{size}/{z}/{lat}/{lon}/{color}/{options}.png
    // Example: /v2/radar/1609402200/512/6/44.31/-69.78/2/1_0.png
    let size = 512;
    let color_scheme = 2; // decent default palette
    let options = "1_0"; // smoothed, no snow mask

    let url = format!(
        "{}{}/{}/{}/{:.4}/{:.4}/{}/{}.png",
        maps.host, frame.path, size, zoom, lat, lon, color_scheme, options
    );

    let bytes = c
        .get(url)
        .send()
        .await
        .context("radar tile request failed")?
        .error_for_status()
        .context("radar tile HTTP error")?
        .bytes()
        .await
        .context("radar tile read failed")?;

    let img = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
        .context("radar tile decode failed")?
        .to_rgba8();

    // Convert to braille: each cell maps to 2x4 subpixels for higher resolution.
    let target_w = 76usize;
    let target_h = 22usize;

    let basemap = fetch_basemap_tiles(lat, lon, zoom, size as usize, target_w * 2, target_h * 4)
        .await
        .ok();
    let cells = rgba_to_braille_cells(&img, target_w, target_h, basemap.as_ref());

    Ok(RadarData {
        fetched_at: Local::now(),
        frame_time_utc: frame.time,
        zoom,
        cells,
        info: "RainViewer radar (past) + basemap".to_string(),
    })
}

fn rgba_to_braille_cells(
    img: &image::RgbaImage,
    w: usize,
    h: usize,
    basemap: Option<&BasemapTiles>,
) -> Vec<Vec<RadarCell>> {
    let iw = img.width().max(1) as usize;
    let ih = img.height().max(1) as usize;
    let sample_w = w.saturating_mul(2).max(1);
    let sample_h = h.saturating_mul(4).max(1);

    let mut out = Vec::with_capacity(h);
    for yy in 0..h {
        let mut line = Vec::with_capacity(w);
        for xx in 0..w {
            let mut bits = [[false; 2]; 4];
            let mut any = false;
            let mut any_radar = false;
            let mut any_border = false;
            for sy in 0..4usize {
                for sx in 0..2usize {
                    let px = (xx * 2 + sx) * iw / sample_w;
                    let py = (yy * 4 + sy) * ih / sample_h;
                    let p = img.get_pixel(px as u32, py as u32).0;

                    let a = p[3] as f32 / 255.0;
                    let r = p[0] as f32 / 255.0;
                    let g = p[1] as f32 / 255.0;
                    let b = p[2] as f32 / 255.0;
                    let radar_lum = (0.2126 * r + 0.7152 * g + 0.0722 * b) * a;
                    let edge = basemap
                        .map(|m| basemap_edge_ink(m, xx * 2 + sx, yy * 4 + sy))
                        .unwrap_or(0.0);

                    let th = bayer_2x4_threshold(xx * 2 + sx, yy * 4 + sy);
                    let radar_on = radar_lum > th;
                    let border_on = edge > 0.22;
                    let on = radar_on || border_on;
                    bits[sy][sx] = on;
                    any |= on;
                    any_radar |= radar_on;
                    any_border |= border_on;
                }
            }
            let ch = if any { braille_from_2x4(bits) } else { ' ' };
            let ink = if any_radar {
                RadarInk::Radar
            } else if any_border {
                RadarInk::Border
            } else {
                RadarInk::None
            };
            line.push(RadarCell { ch, ink });
        }
        out.push(line);
    }
    out
}

fn bayer_2x4_threshold(ix: usize, iy: usize) -> f32 {
    const M: [[u8; 2]; 4] = [[0, 4], [6, 2], [1, 5], [7, 3]];
    let v = M[iy & 3][ix & 1] as f32;
    (v + 0.5) / 8.0
}

fn braille_from_2x4(bits: [[bool; 2]; 4]) -> char {
    let mut mask = 0u16;
    if bits[0][0] { mask |= 1 << 0; }
    if bits[1][0] { mask |= 1 << 1; }
    if bits[2][0] { mask |= 1 << 2; }
    if bits[0][1] { mask |= 1 << 3; }
    if bits[1][1] { mask |= 1 << 4; }
    if bits[2][1] { mask |= 1 << 5; }
    if bits[3][0] { mask |= 1 << 6; }
    if bits[3][1] { mask |= 1 << 7; }
    std::char::from_u32(0x2800 + mask as u32).unwrap_or(' ')
}

async fn fetch_basemap_tiles(
    lat: f64,
    lon: f64,
    zoom: u8,
    radar_size: usize,
    sample_w: usize,
    sample_h: usize,
) -> Result<BasemapTiles> {
    let tile_size = 256.0;
    let n = 2_i32.pow(zoom as u32) as i32;
    let (center_x, center_y) = lat_lon_to_world_px(lat, lon, zoom, tile_size);
    let radar_size = radar_size as f64;
    let top_left_world_x = center_x - radar_size * 0.5;
    let top_left_world_y = center_y - radar_size * 0.5;
    let scale_x = radar_size / sample_w as f64;
    let scale_y = radar_size / sample_h as f64;

    let min_tx = (top_left_world_x / tile_size).floor() as i32;
    let max_tx = ((top_left_world_x + radar_size) / tile_size).floor() as i32;
    let min_ty = (top_left_world_y / tile_size).floor() as i32;
    let max_ty = ((top_left_world_y + radar_size) / tile_size).floor() as i32;

    let c = reqwest::Client::new();
    let mut tiles = HashMap::new();
    for ty in min_ty..=max_ty {
        if ty < 0 || ty >= n {
            continue;
        }
        for tx in min_tx..=max_tx {
            let wx = wrap_tile_x(tx, n);
            let url = format!(
                "https://a.basemaps.cartocdn.com/rastertiles/voyager_nolabels/{}/{}/{}.png",
                zoom, wx, ty
            );
            let bytes = c
                .get(url)
                .send()
                .await
                .context("basemap tile request failed")?
                .error_for_status()
                .context("basemap tile HTTP error")?
                .bytes()
                .await
                .context("basemap tile read failed")?;
            let tile = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
                .context("basemap tile decode failed")?
                .to_rgba8();
            tiles.insert((wx, ty), tile);
        }
    }

    Ok(BasemapTiles {
        tiles,
        zoom,
        top_left_world_x,
        top_left_world_y,
        scale_x,
        scale_y,
        sample_w,
        sample_h,
    })
}

fn basemap_luma(map: &BasemapTiles, sx: usize, sy: usize) -> Option<f32> {
    if sx >= map.sample_w || sy >= map.sample_h {
        return None;
    }
    let tile_size = 256.0;
    let n = 2_i32.pow(map.zoom as u32) as i32;
    let wx = map.top_left_world_x + sx as f64 * map.scale_x;
    let wy = map.top_left_world_y + sy as f64 * map.scale_y;
    let tx = (wx / tile_size).floor() as i32;
    let ty = (wy / tile_size).floor() as i32;
    if ty < 0 || ty >= n {
        return None;
    }
    let tx = wrap_tile_x(tx, n);
    let ox = (wx - (tx as f64) * tile_size).floor() as i32;
    let oy = (wy - (ty as f64) * tile_size).floor() as i32;
    let tile = map.tiles.get(&(tx, ty))?;
    let px = ox.clamp(0, 255) as u32;
    let py = oy.clamp(0, 255) as u32;
    let p = tile.get_pixel(px, py).0;
    let a = p[3] as f32 / 255.0;
    let r = p[0] as f32 / 255.0;
    let g = p[1] as f32 / 255.0;
    let b = p[2] as f32 / 255.0;
    Some((0.2126 * r + 0.7152 * g + 0.0722 * b) * a)
}

fn basemap_edge_ink(map: &BasemapTiles, sx: usize, sy: usize) -> f32 {
    let l0 = basemap_luma(map, sx, sy).unwrap_or(0.0);
    let lx = basemap_luma(map, sx + 1, sy).unwrap_or(l0);
    let ly = basemap_luma(map, sx, sy + 1).unwrap_or(l0);
    let edge = ((l0 - lx).abs() + (l0 - ly).abs()) * 1.8;
    ((edge - 0.08).max(0.0) * 3.5).min(1.0)
}

fn lat_lon_to_world_px(lat: f64, lon: f64, zoom: u8, tile_size: f64) -> (f64, f64) {
    let lat = lat.clamp(-85.0511, 85.0511);
    let n = 2.0_f64.powi(zoom as i32);
    let x = (lon + 180.0) / 360.0;
    let lat_rad = lat.to_radians();
    let y = (1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI) / 2.0;
    (x * n * tile_size, y * n * tile_size)
}

fn wrap_tile_x(tx: i32, n: i32) -> i32 {
    ((tx % n) + n) % n
}
