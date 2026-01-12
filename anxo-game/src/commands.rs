use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Command {
    Move(Direction),
}

impl Command {
    pub fn to_wire(self) -> String {
        match self {
            Command::Move(Direction::Up) => "move_up".to_string(),
            Command::Move(Direction::Down) => "move_down".to_string(),
            Command::Move(Direction::Left) => "move_left".to_string(),
            Command::Move(Direction::Right) => "move_right".to_string(),
        }
    }

    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            "move_up" => Some(Command::Move(Direction::Up)),
            "move_down" => Some(Command::Move(Direction::Down)),
            "move_left" => Some(Command::Move(Direction::Left)),
            "move_right" => Some(Command::Move(Direction::Right)),
            _ => None,
        }
    }
}
