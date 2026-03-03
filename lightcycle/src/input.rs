use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use crate::sim::Dir;

#[derive(Debug, Clone, Copy)]
pub enum InputAction {
    Quit,
    Restart,
    PauseToggle,
    ToggleWalls,
    DropWall,
}

#[derive(Default, Debug)]
pub struct InputState {
    pub buffered_dir: Option<Dir>,
    pub actions: Vec<InputAction>,
    pub resized: bool,
}

impl InputState {
    pub fn clear_frame_flags(&mut self) {
        self.actions.clear();
        self.resized = false;
    }

    pub fn take_buffered_dir(&mut self) -> Option<Dir> {
        self.buffered_dir.take()
    }
}

pub fn poll_input(state: &mut InputState) -> std::io::Result<()> {
    while event::poll(Duration::ZERO)? {
        match event::read()? {
            Event::Resize(_, _) => {
                state.resized = true;
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    state.actions.push(InputAction::Quit);
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') => state.actions.push(InputAction::Quit),
                    KeyCode::Char('r') => state.actions.push(InputAction::Restart),
                    KeyCode::Char('v') => state.actions.push(InputAction::ToggleWalls),
                    KeyCode::Char('e') => state.actions.push(InputAction::DropWall),
                    KeyCode::Char(' ') => state.actions.push(InputAction::PauseToggle),
                    KeyCode::Char('w') | KeyCode::Up => state.buffered_dir = Some(Dir::Up),
                    KeyCode::Char('s') | KeyCode::Down => state.buffered_dir = Some(Dir::Down),
                    KeyCode::Char('a') | KeyCode::Left => state.buffered_dir = Some(Dir::Left),
                    KeyCode::Char('d') | KeyCode::Right => state.buffered_dir = Some(Dir::Right),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(())
}
