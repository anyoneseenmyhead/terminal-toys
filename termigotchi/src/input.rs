use crate::model::Scene;
use crate::sim::PlayerAction;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::time::Duration;

#[derive(Clone, Debug)]
pub(crate) struct InputEvent {
    pub(crate) key: KeyCode,
    pub(crate) mods: KeyModifiers,
}

pub(crate) fn collect_input_nonblocking(max_frame_time: Duration) -> anyhow::Result<Vec<InputEvent>> {
    let mut out = Vec::new();

    // poll with a tiny timeout so we stay responsive
    let timeout = std::cmp::min(Duration::from_millis(1), max_frame_time);
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

pub(crate) fn map_event_to_action(scene: &Scene, ev: InputEvent) -> Option<PlayerAction> {
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
