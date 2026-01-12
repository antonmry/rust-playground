mod commands;
mod level;
mod python;
mod ui;

use std::time::Duration;

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{Projection, ScalingMode, Viewport};
use bevy::prelude::*;
use bevy::window::WindowResolution;
use bevy_egui::{
    EguiContext, EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, PrimaryEguiContext,
};
use crossbeam_channel::{Receiver, TryRecvError};

use commands::{Command, Direction};
use level::{LevelAssets, LevelMap, TILE_SIZE, grid_to_world, parse_level};
use ui::{EditorState, ResetRequest, RunRequest};

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, Default)]
enum GamePhase {
    #[default]
    Editing,
    Playing,
    Won,
}

#[derive(Component)]
struct Hero {
    grid_pos: IVec2,
}

#[derive(Component)]
struct Door;

#[derive(Component)]
struct Moving {
    start: Vec3,
    end: Vec3,
    target_grid: IVec2,
    timer: Timer,
}

#[derive(Component)]
struct ResetAnim {
    total: Timer,
    frame: Timer,
    index: usize,
    base_pos: Vec3,
}

type HeroQueryData = (
    Entity,
    &'static mut Hero,
    &'static Transform,
    Option<&'static Moving>,
    Option<&'static ResetAnim>,
);

#[derive(Component)]
struct WorldCamera;

#[derive(Resource, Default)]
struct CommandQueue {
    commands: Vec<Command>,
    index: usize,
}

#[derive(Resource)]
struct PlaybackTimer(Timer);

#[derive(Resource, Default)]
struct PythonTask {
    receiver: Option<Receiver<Result<Vec<Command>, String>>>,
    running: bool,
}

#[derive(Resource, Default)]
struct AutoRun {
    done: bool,
}

#[derive(Resource)]
pub struct UiLayout {
    editor_width: f32,
    editor_left: f32,
    pixels_per_point: f32,
}

#[derive(Resource, Clone, Copy)]
struct PlaceholderMode(bool);

#[derive(Resource, Default)]
struct RunState {
    has_run: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--python-worker") {
        std::process::exit(python::run_worker());
    }

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Anxo Game".to_string(),
                resolution: WindowResolution::new(960, 540),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.08, 0.08, 0.1)))
        .insert_resource(EguiGlobalSettings {
            auto_create_primary_context: false,
            ..Default::default()
        })
        .add_plugins(EguiPlugin::default())
        .insert_resource(CommandQueue::default())
        .insert_resource(PlaybackTimer(Timer::from_seconds(
            0.2,
            TimerMode::Repeating,
        )))
        .insert_resource(PythonTask::default())
        .insert_resource(GamePhase::default())
        .insert_resource(UiLayout {
            editor_width: 360.0,
            editor_left: 600.0,
            pixels_per_point: 1.0,
        })
        .insert_resource(EditorState {
            code: initial_code(),
            error: None,
        })
        .insert_resource(AutoRun::default())
        .insert_resource(RunState::default())
        .add_message::<RunRequest>()
        .add_message::<ResetRequest>()
        .add_systems(Startup, setup)
        .add_systems(EguiPrimaryContextPass, ui::ui_system)
        .add_systems(
            PreUpdate,
            (
                auto_run_system,
                handle_run_requests,
                poll_python_results,
                update_camera_viewport,
                reset_animation_system,
                playback_system,
                movement_system,
                win_system,
                reset_system,
            ),
        )
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    let level_text = include_str!("../assets/levels/level1.txt");
    let level = parse_level(level_text);
    let use_placeholders = std::env::var("ANXO_PLACEHOLDER").ok().as_deref() == Some("1");
    let world_layer = RenderLayers::layer(0);
    let assets = LevelAssets {
        floor: asset_server.load("kenney_pixel_platformer/Tiles/Backgrounds/tile_0000.png"),
        wall: asset_server.load("kenney_pixel_platformer/Tiles/tile_0001.png"),
        door: asset_server.load("kenney_pixel_platformer/Tiles/tile_0017.png"),
        hero: asset_server.load("kenney_pixel_platformer/Tiles/Characters/tile_0000.png"),
        hero_frames: vec![
            asset_server.load("kenney_pixel_platformer/Tiles/Characters/tile_0000.png"),
            asset_server.load("kenney_pixel_platformer/Tiles/Characters/tile_0001.png"),
            asset_server.load("kenney_pixel_platformer/Tiles/Characters/tile_0002.png"),
            asset_server.load("kenney_pixel_platformer/Tiles/Characters/tile_0003.png"),
        ],
    };

    let camera_center = Vec3::new(
        (level.width as f32 - 1.0) * TILE_SIZE * 0.5,
        (level.height as f32 - 1.0) * TILE_SIZE * 0.5,
        999.0,
    );
    commands.spawn((
        Camera2d,
        Camera {
            order: 0,
            ..Default::default()
        },
        Transform::from_translation(camera_center),
        WorldCamera,
        world_layer.clone(),
    ));
    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..Default::default()
        },
        RenderLayers::layer(1),
        EguiContext::default(),
        PrimaryEguiContext,
    ));

    for y in 0..level.height {
        for x in 0..level.width {
            let pos = IVec2::new(x, y);
            let world_pos = grid_to_world(pos);
            commands.spawn((
                if use_placeholders {
                    Sprite {
                        color: Color::srgb(0.2, 0.22, 0.25),
                        custom_size: Some(Vec2::splat(TILE_SIZE)),
                        ..Default::default()
                    }
                } else {
                    Sprite {
                        image: assets.floor.clone(),
                        custom_size: Some(Vec2::splat(TILE_SIZE)),
                        ..Default::default()
                    }
                },
                Transform::from_translation(world_pos),
                world_layer.clone(),
            ));
        }
    }

    for wall_pos in &level.walls {
        commands.spawn((
            if use_placeholders {
                Sprite {
                    color: Color::srgb(0.12, 0.12, 0.12),
                    custom_size: Some(Vec2::splat(TILE_SIZE)),
                    ..Default::default()
                }
            } else {
                Sprite {
                    image: assets.wall.clone(),
                    custom_size: Some(Vec2::splat(TILE_SIZE)),
                    ..Default::default()
                }
            },
            Transform::from_translation(grid_to_world(*wall_pos) + Vec3::new(0.0, 0.0, 1.0)),
            world_layer.clone(),
        ));
    }

    commands.spawn((
        if use_placeholders {
            Sprite {
                color: Color::srgb(0.2, 0.6, 0.25),
                custom_size: Some(Vec2::splat(TILE_SIZE)),
                ..Default::default()
            }
        } else {
            Sprite {
                image: assets.door.clone(),
                custom_size: Some(Vec2::splat(TILE_SIZE)),
                ..Default::default()
            }
        },
        Transform::from_translation(grid_to_world(level.door) + Vec3::new(0.0, 0.0, 1.0)),
        world_layer.clone(),
        Door,
    ));

    commands.spawn((
        if use_placeholders {
            Sprite {
                color: Color::srgb(0.9, 0.75, 0.2),
                custom_size: Some(Vec2::splat(TILE_SIZE)),
                ..Default::default()
            }
        } else {
            Sprite {
                image: assets.hero.clone(),
                custom_size: Some(Vec2::splat(TILE_SIZE)),
                ..Default::default()
            }
        },
        Transform::from_translation(grid_to_world(level.hero_start) + Vec3::new(0.0, 0.0, 2.0)),
        world_layer,
        Hero {
            grid_pos: level.hero_start,
        },
    ));

    commands.insert_resource(level);
    commands.insert_resource(assets);
    commands.insert_resource(PlaceholderMode(use_placeholders));
}

#[allow(clippy::too_many_arguments)]
fn handle_run_requests(
    mut events: MessageReader<RunRequest>,
    mut python_task: ResMut<PythonTask>,
    mut editor: ResMut<EditorState>,
    level: Res<LevelMap>,
    mut command_queue: ResMut<CommandQueue>,
    mut phase: ResMut<GamePhase>,
    mut hero_query: Query<(Entity, &mut Hero, &mut Transform, Option<&Moving>)>,
    mut commands: Commands,
    run_state: ResMut<RunState>,
) {
    if python_task.running {
        events.clear();
        return;
    }

    if let Some(event) = events.read().last() {
        if run_state.has_run {
            reset_game_state(
                &level,
                &mut command_queue,
                &mut phase,
                &mut editor,
                &mut hero_query,
                &mut commands,
                true,
            );
        } else {
            reset_game_state(
                &level,
                &mut command_queue,
                &mut phase,
                &mut editor,
                &mut hero_query,
                &mut commands,
                false,
            );
        }
        let code = event.0.clone();
        let (tx, rx) = crossbeam_channel::unbounded();
        std::thread::spawn(move || {
            let result = python::run_code_via_worker(code, Duration::from_secs(1));
            let _ = tx.send(result);
        });
        python_task.receiver = Some(rx);
        python_task.running = true;
    }
}

fn auto_run_system(
    mut autorun: ResMut<AutoRun>,
    editor: Res<EditorState>,
    mut run_events: MessageWriter<RunRequest>,
) {
    if autorun.done {
        return;
    }
    if std::env::var("ANXO_AUTORUN").ok().as_deref() != Some("1") {
        return;
    }
    run_events.write(RunRequest(editor.code.clone()));
    autorun.done = true;
}

#[allow(clippy::too_many_arguments)]
fn poll_python_results(
    mut python_task: ResMut<PythonTask>,
    mut command_queue: ResMut<CommandQueue>,
    mut phase: ResMut<GamePhase>,
    mut editor: ResMut<EditorState>,
    _level: Res<LevelMap>,
    _hero_query: Query<(Entity, &mut Hero, &mut Transform, Option<&Moving>)>,
    _commands: Commands,
    mut run_state: ResMut<RunState>,
) {
    let Some(receiver) = python_task.receiver.as_ref() else {
        return;
    };

    match receiver.try_recv() {
        Ok(result) => {
            python_task.running = false;
            python_task.receiver = None;
            match result {
                Ok(parsed_commands) => {
                    command_queue.commands = parsed_commands;
                    command_queue.index = 0;
                    *phase = GamePhase::Playing;
                    run_state.has_run = true;
                }
                Err(error) => {
                    editor.error = Some(error);
                    *phase = GamePhase::Editing;
                }
            }
        }
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => {
            python_task.running = false;
            python_task.receiver = None;
            editor.error = Some("Python worker disconnected".to_string());
        }
    }
}

fn initial_code() -> String {
    if let Ok(code) = std::env::var("ANXO_START_CODE") {
        return code;
    }
    "from game import hero\n".to_string()
}

#[allow(clippy::too_many_arguments)]
fn playback_system(
    time: Res<Time>,
    mut timer: ResMut<PlaybackTimer>,
    level: Res<LevelMap>,
    mut command_queue: ResMut<CommandQueue>,
    phase: Res<GamePhase>,
    mut editor: ResMut<EditorState>,
    mut hero_query: Query<HeroQueryData>,
    mut commands: Commands,
) {
    if *phase != GamePhase::Playing {
        return;
    }
    timer.0.tick(time.delta());
    if !timer.0.is_finished() {
        return;
    }

    let Ok((hero_entity, hero, transform, moving, reset_anim)) = hero_query.single_mut() else {
        return;
    };

    if moving.is_some() {
        return;
    }
    if reset_anim.is_some() {
        return;
    }

    let Some(command) = command_queue.commands.get(command_queue.index) else {
        return;
    };

    let mut target = hero.grid_pos;
    match command {
        Command::Move(Direction::Up) => target.y += 1,
        Command::Move(Direction::Down) => target.y -= 1,
        Command::Move(Direction::Left) => target.x -= 1,
        Command::Move(Direction::Right) => target.x += 1,
    }

    if target.x < 0 || target.y < 0 || target.x >= level.width || target.y >= level.height {
        editor.error = Some("You can't walk through walls.".to_string());
        return;
    }

    if level.is_wall(target) {
        editor.error = Some("You can't walk through walls.".to_string());
        return;
    }

    let start = transform.translation;
    let end = grid_to_world(target) + Vec3::new(0.0, 0.0, 2.0);
    commands.entity(hero_entity).insert(Moving {
        start,
        end,
        target_grid: target,
        timer: Timer::from_seconds(0.15, TimerMode::Once),
    });

    command_queue.index += 1;
}

fn movement_system(
    time: Res<Time>,
    mut commands: Commands,
    mut hero_query: Query<(Entity, &mut Hero, &mut Transform, &mut Moving)>,
) {
    let Ok((entity, mut hero, mut transform, mut moving)) = hero_query.single_mut() else {
        return;
    };

    moving.timer.tick(time.delta());
    let duration = moving.timer.duration().as_secs_f32().max(f32::EPSILON);
    let t = (moving.timer.elapsed_secs() / duration).clamp(0.0, 1.0);
    transform.translation = moving.start.lerp(moving.end, t);

    if moving.timer.is_finished() {
        hero.grid_pos = moving.target_grid;
        commands.entity(entity).remove::<Moving>();
    }
}

fn win_system(
    hero_query: Query<(&Hero, Option<&Moving>)>,
    level: Res<LevelMap>,
    mut phase: ResMut<GamePhase>,
) {
    if *phase != GamePhase::Playing {
        return;
    }
    let Ok((hero, moving)) = hero_query.single() else {
        return;
    };

    if moving.is_none() && hero.grid_pos == level.door {
        *phase = GamePhase::Won;
    }
}

#[allow(clippy::too_many_arguments)]
fn reset_system(
    mut events: MessageReader<ResetRequest>,
    level: Res<LevelMap>,
    mut command_queue: ResMut<CommandQueue>,
    mut phase: ResMut<GamePhase>,
    mut editor: ResMut<EditorState>,
    mut hero_query: Query<(Entity, &mut Hero, &mut Transform, Option<&Moving>)>,
    mut commands: Commands,
    mut run_state: ResMut<RunState>,
) {
    if events.is_empty() {
        return;
    }
    events.clear();

    reset_game_state(
        &level,
        &mut command_queue,
        &mut phase,
        &mut editor,
        &mut hero_query,
        &mut commands,
        true,
    );
    run_state.has_run = false;
}

fn reset_game_state(
    level: &LevelMap,
    command_queue: &mut CommandQueue,
    phase: &mut GamePhase,
    editor: &mut EditorState,
    hero_query: &mut Query<(Entity, &mut Hero, &mut Transform, Option<&Moving>)>,
    commands: &mut Commands,
    animate: bool,
) {
    command_queue.commands.clear();
    command_queue.index = 0;
    editor.error = None;
    *phase = GamePhase::Editing;

    if let Ok((entity, mut hero, mut transform, _)) = hero_query.single_mut() {
        hero.grid_pos = level.hero_start;
        transform.translation = grid_to_world(level.hero_start) + Vec3::new(0.0, 0.0, 2.0);
        commands.entity(entity).remove::<Moving>();
        if animate {
            trigger_reset_animation(level, hero_query, commands);
        }
    }
}

fn trigger_reset_animation(
    level: &LevelMap,
    hero_query: &mut Query<(Entity, &mut Hero, &mut Transform, Option<&Moving>)>,
    commands: &mut Commands,
) {
    if let Ok((entity, mut hero, mut transform, _)) = hero_query.single_mut() {
        hero.grid_pos = level.hero_start;
        transform.translation = grid_to_world(level.hero_start) + Vec3::new(0.0, 0.0, 2.0);
        commands.entity(entity).remove::<Moving>();
        commands.entity(entity).insert(ResetAnim {
            total: Timer::from_seconds(0.8, TimerMode::Once),
            frame: Timer::from_seconds(0.08, TimerMode::Repeating),
            index: 0,
            base_pos: transform.translation,
        });
    }
}

fn reset_animation_system(
    time: Res<Time>,
    assets: Res<LevelAssets>,
    placeholder: Res<PlaceholderMode>,
    mut commands: Commands,
    mut hero_query: Query<(Entity, &mut Sprite, &mut Transform, &mut ResetAnim)>,
) {
    for (entity, mut sprite, mut transform, mut anim) in &mut hero_query {
        anim.total.tick(time.delta());
        anim.frame.tick(time.delta());
        let t = anim.total.elapsed_secs() / anim.total.duration().as_secs_f32().max(f32::EPSILON);
        let jump = (t * std::f32::consts::PI).sin() * 16.0;
        transform.translation = anim.base_pos + Vec3::new(0.0, jump, 0.0);
        if anim.frame.just_finished() {
            anim.index = anim.index.saturating_add(1);
            if placeholder.0 {
                let pulse = if anim.index % 2 == 0 { 1.0 } else { 0.5 };
                sprite.color = Color::srgb(pulse, pulse * 0.75, 0.15);
            } else if !assets.hero_frames.is_empty() {
                let frame_index = anim.index % assets.hero_frames.len();
                sprite.image = assets.hero_frames[frame_index].clone();
                sprite.custom_size = Some(Vec2::splat(TILE_SIZE));
            }
        }

        if anim.total.is_finished() {
            if !placeholder.0 {
                sprite.image = assets.hero.clone();
                sprite.custom_size = Some(Vec2::splat(TILE_SIZE));
            }
            transform.translation = anim.base_pos;
            commands.entity(entity).remove::<ResetAnim>();
        }
    }
}

fn update_camera_viewport(
    windows: Query<&Window>,
    layout: Res<UiLayout>,
    level: Res<LevelMap>,
    mut camera_query: Query<(&mut Camera, &mut Projection, &mut Transform), With<WorldCamera>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Ok((mut camera, mut projection, mut transform)) = camera_query.single_mut() else {
        return;
    };

    let scale = layout.pixels_per_point.max(0.1);
    let editor_px = (layout.editor_width * scale).round() as u32;
    let window_size = window.physical_size();
    let viewport_width = window_size.x.saturating_sub(editor_px);

    if viewport_width == 0 || window_size.y == 0 {
        camera.viewport = None;
        return;
    }
    camera.viewport = Some(Viewport {
        physical_position: UVec2::new(0, 0),
        physical_size: UVec2::new(viewport_width, window_size.y),
        ..Default::default()
    });

    let Projection::Orthographic(ref mut ortho) = *projection else {
        return;
    };
    let viewport_aspect = viewport_width as f32 / window_size.y as f32;
    let level_w = level.width as f32 * TILE_SIZE;
    let level_h = level.height as f32 * TILE_SIZE;
    let level_aspect = level_w / level_h.max(f32::EPSILON);
    let viewport_height = if viewport_aspect > level_aspect {
        level_w / viewport_aspect
    } else {
        level_h
    };
    ortho.scaling_mode = ScalingMode::FixedVertical { viewport_height };
    ortho.scale = 1.0;

    transform.translation = Vec3::new(
        (level.width as f32 - 1.0) * TILE_SIZE * 0.5,
        (level.height as f32 - 1.0) * TILE_SIZE * 0.5,
        999.0,
    );
}
