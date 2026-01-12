use std::collections::HashSet;

use bevy::prelude::*;

pub const TILE_SIZE: f32 = 32.0;

#[derive(Resource, Clone)]
pub struct LevelMap {
    pub width: i32,
    pub height: i32,
    pub walls: HashSet<IVec2>,
    pub door: IVec2,
    pub hero_start: IVec2,
}

impl LevelMap {
    pub fn is_wall(&self, pos: IVec2) -> bool {
        self.walls.contains(&pos)
    }
}

#[derive(Resource, Clone)]
pub struct LevelAssets {
    pub floor: Handle<Image>,
    pub wall: Handle<Image>,
    pub door: Handle<Image>,
    pub hero: Handle<Image>,
    pub hero_frames: Vec<Handle<Image>>,
}

pub fn parse_level(text: &str) -> LevelMap {
    let mut walls = HashSet::new();
    let mut door = IVec2::ZERO;
    let mut hero_start = IVec2::ZERO;
    let mut width = 0;

    let lines: Vec<&str> = text.lines().collect();
    let height = lines.len() as i32;
    for (row, line) in lines.iter().enumerate() {
        width = width.max(line.chars().count() as i32);
        for (col, ch) in line.chars().enumerate() {
            let pos = IVec2::new(col as i32, height - 1 - row as i32);
            match ch {
                '#' => {
                    walls.insert(pos);
                }
                'D' => {
                    door = pos;
                }
                'H' => {
                    hero_start = pos;
                }
                _ => {}
            }
        }
    }

    LevelMap {
        width,
        height,
        walls,
        door,
        hero_start,
    }
}

pub fn grid_to_world(pos: IVec2) -> Vec3 {
    Vec3::new(pos.x as f32 * TILE_SIZE, pos.y as f32 * TILE_SIZE, 0.0)
}
