use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustpython_vm::Interpreter;
use rustpython_vm::Settings;
use rustpython_vm::VirtualMachine;
use rustpython_vm::builtins::PyBaseExceptionRef;
use serde::{Deserialize, Serialize};

use crate::commands::{Command, Direction};

const MAX_COMMANDS: usize = 200;

#[derive(Debug, Serialize, Deserialize)]
struct WorkerResponse {
    ok: bool,
    commands: Vec<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EvalRequest {
    code: String,
    context_literal: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct EvalResponse {
    ok: bool,
    error: Option<String>,
}

pub fn run_code_via_worker(code: String, timeout: Duration) -> Result<Vec<Command>, String> {
    let exe_path = std::env::current_exe().map_err(|err| err.to_string())?;
    let mut child = ProcessCommand::new(exe_path)
        .arg("--python-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| err.to_string())?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(code.as_bytes())
            .map_err(|err| err.to_string())?;
    }

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("Python execution timed out".to_string());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => return Err(err.to_string()),
        }
    }

    let output = child.wait_with_output().map_err(|err| err.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(format!("Python worker failed: {stderr}"));
    }

    let response: WorkerResponse = serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("Failed to parse worker output: {err}"))?;

    if response.ok {
        let mut commands = Vec::new();
        for wire in response.commands {
            let command = Command::from_wire(&wire)
                .ok_or_else(|| format!("Unknown command from worker: {wire}"))?;
            commands.push(command);
        }
        Ok(commands)
    } else {
        Err(response.error.unwrap_or_else(|| "Python error".to_string()))
    }
}

pub fn run_eval_via_worker(
    code: String,
    context_literal: String,
    timeout: Duration,
) -> Result<(), String> {
    let exe_path = std::env::current_exe().map_err(|err| err.to_string())?;
    let project_root = std::env::var("ANXO_PROJECT_ROOT")
        .unwrap_or_else(|_| env!("CARGO_MANIFEST_DIR").to_string());
    let eval_lib = Path::new(&project_root)
        .join("assets")
        .join("levels")
        .join("_lib");
    let mut child = ProcessCommand::new(exe_path)
        .arg("--python-eval-worker")
        .env("ANXO_PROJECT_ROOT", project_root)
        .env("ANXO_EVAL_LIB", eval_lib)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| err.to_string())?;

    let request = EvalRequest {
        code,
        context_literal,
    };
    if let Some(mut stdin) = child.stdin.take() {
        let payload =
            serde_json::to_vec(&request).map_err(|err| format!("Eval request error: {err}"))?;
        stdin.write_all(&payload).map_err(|err| err.to_string())?;
    }

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("Evaluation timed out".to_string());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => return Err(err.to_string()),
        }
    }

    let output = child.wait_with_output().map_err(|err| err.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(format!("Evaluation worker failed: {stderr}"));
    }

    let response: EvalResponse = serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("Failed to parse evaluation output: {err}"))?;

    if response.ok {
        Ok(())
    } else {
        Err(response
            .error
            .unwrap_or_else(|| "Evaluation failed".to_string()))
    }
}

pub fn run_worker() -> i32 {
    let mut code = String::new();
    if std::io::stdin().read_to_string(&mut code).is_err() {
        return 1;
    }

    let response = match run_python(&code) {
        Ok(commands) => WorkerResponse {
            ok: true,
            commands: commands.into_iter().map(|cmd| cmd.to_wire()).collect(),
            error: None,
        },
        Err(error) => WorkerResponse {
            ok: false,
            commands: Vec::new(),
            error: Some(error),
        },
    };

    let output = serde_json::to_string(&response).unwrap_or_else(|_| {
        "{\"ok\":false,\"commands\":[],\"error\":\"Worker serialization failed\"}".to_string()
    });
    println!("{output}");
    0
}

pub fn run_eval_worker() -> i32 {
    let mut payload = String::new();
    if std::io::stdin().read_to_string(&mut payload).is_err() {
        return 1;
    }

    let request: EvalRequest = match serde_json::from_str(&payload) {
        Ok(request) => request,
        Err(err) => {
            let output = serde_json::to_string(&EvalResponse {
                ok: false,
                error: Some(format!("Invalid eval request: {err}")),
            })
            .unwrap_or_else(|_| {
                "{\"ok\":false,\"error\":\"Eval request serialization failed\"}".to_string()
            });
            println!("{output}");
            return 0;
        }
    };

    let response = match run_eval(&request.code, &request.context_literal) {
        Ok(()) => EvalResponse { ok: true, error: None },
        Err(error) => EvalResponse {
            ok: false,
            error: Some(error),
        },
    };

    let output = serde_json::to_string(&response).unwrap_or_else(|_| {
        "{\"ok\":false,\"error\":\"Eval response serialization failed\"}".to_string()
    });
    println!("{output}");
    0
}

fn run_python(code: &str) -> Result<Vec<Command>, String> {
    let commands = Arc::new(Mutex::new(Vec::new()));
    let interpreter = Interpreter::with_init(Settings::default(), |_vm| {});
    let result = interpreter.enter(|vm| run_with_vm(vm, code, commands.clone()));

    match result {
        Ok(()) => Ok(commands
            .lock()
            .map_err(|_| "Command buffer locked".to_string())?
            .clone()),
        Err(err) => Err(err),
    }
}

fn run_eval(code: &str, context_literal: &str) -> Result<(), String> {
    let interpreter = Interpreter::with_init(Settings::default(), |_vm| {});
    let result = interpreter.enter(|vm| run_eval_with_vm(vm, code, context_literal));
    match result {
        Ok(()) => Ok(()),
        Err(err) => Err(err),
    }
}

fn run_with_vm(
    vm: &VirtualMachine,
    code: &str,
    commands: Arc<Mutex<Vec<Command>>>,
) -> Result<(), String> {
    let scope = vm.new_scope_with_builtins();

    let commands_for_moves = commands.clone();
    let record_fn = vm.new_function(
        "__record_move",
        move |direction: String, vm: &VirtualMachine| {
            let command = match direction.as_str() {
                "up" => Command::Move(Direction::Up),
                "down" => Command::Move(Direction::Down),
                "left" => Command::Move(Direction::Left),
                "right" => Command::Move(Direction::Right),
                _ => {
                    let err = vm.new_exception_msg(
                        vm.ctx.exceptions.value_error.to_owned(),
                        format!("Unknown direction: {direction}"),
                    );
                    return Err(err);
                }
            };
            let mut buffer = commands_for_moves.lock().map_err(|_| {
                vm.new_exception_msg(
                    vm.ctx.exceptions.runtime_error.to_owned(),
                    "Command buffer locked".to_string(),
                )
            })?;
            if buffer.len() >= MAX_COMMANDS {
                let err = vm.new_exception_msg(
                    vm.ctx.exceptions.runtime_error.to_owned(),
                    format!("Too many commands (max {MAX_COMMANDS})"),
                );
                return Err(err);
            }
            buffer.push(command);
            Ok(())
        },
    );
    let commands_for_actions = commands.clone();
    let action_fn = vm.new_function(
        "__record_action",
        move |action: String, vm: &VirtualMachine| {
            let command = match action.as_str() {
                "pick" => Command::Pick,
                "open" => Command::Open,
                _ => {
                    let err = vm.new_exception_msg(
                        vm.ctx.exceptions.value_error.to_owned(),
                        format!("Unknown action: {action}"),
                    );
                    return Err(err);
                }
            };
            let mut buffer = commands_for_actions.lock().map_err(|_| {
                vm.new_exception_msg(
                    vm.ctx.exceptions.runtime_error.to_owned(),
                    "Command buffer locked".to_string(),
                )
            })?;
            if buffer.len() >= MAX_COMMANDS {
                let err = vm.new_exception_msg(
                    vm.ctx.exceptions.runtime_error.to_owned(),
                    format!("Too many commands (max {MAX_COMMANDS})"),
                );
                return Err(err);
            }
            buffer.push(command);
            Ok(())
        },
    );

    scope
        .globals
        .set_item("__record_move", record_fn.into(), vm)
        .map_err(|err| format_python_error(vm, &err))?;
    scope
        .globals
        .set_item("__record_action", action_fn.into(), vm)
        .map_err(|err| format_python_error(vm, &err))?;

    let prelude = r#"
import sys

class _Game:
    pass

class _Key:
    pass

class _Padlock:
    pass

class _Hero:
    def __init__(self, recorder):
        self._recorder = recorder

    def move_up(self):
        self._recorder("up")

    def move_down(self):
        self._recorder("down")

    def move_left(self):
        self._recorder("left")

    def move_right(self):
        self._recorder("right")

    def pick(self, obj):
        _record_action("pick")

    def open(self, obj):
        _record_action("open")

_record_action = __record_action
_key = _Key()
_padlock = _Padlock()

_game = _Game()
_game.hero = _Hero(__record_move)
_game.key = _key
_game.padlock = _padlock

sys.modules["game"] = _game
"#;

    vm.run_code_string(scope.clone(), prelude, "<prelude>".to_string())
        .map_err(|err| format_python_error(vm, &err))?;
    vm.run_code_string(scope, code, "<user>".to_string())
        .map_err(|err| format_python_error(vm, &err))?;

    Ok(())
}

fn run_eval_with_vm(
    vm: &VirtualMachine,
    code: &str,
    context_literal: &str,
) -> Result<(), String> {
    let scope = vm.new_scope_with_builtins();
    let mut prelude = String::new();
    prelude.push_str(
        r#"import sys

class Hero:
    def __init__(self, x, y, steps, last_move):
        self.x = x
        self.y = y
        self.steps = steps
        self.last_move = last_move

    def pos(self):
        return (self.x, self.y)

    def at_flag(self, level):
        return self.x == level.flag_pos[0] and self.y == level.flag_pos[1]

class Level:
    def __init__(self, width, height, flag_pos, walls):
        self.width = width
        self.height = height
        self.flag_pos = flag_pos
        self._walls = set(tuple(pos) for pos in walls)

    def is_wall(self, x, y):
        return (x, y) in self._walls

class CommandLog:
    def __init__(self, commands):
        self.list = list(commands)

    @property
    def count(self):
        return len(self.list)

class Events:
    def __init__(self, reached_flag, blocked_moves, errors, key_collected, lock_unlocked):
        self.reached_flag = reached_flag
        self.blocked_moves = list(blocked_moves)
        self.errors = list(errors)
        self.key_collected = key_collected
        self.lock_unlocked = lock_unlocked

class EvalContext:
    def __init__(self, hero, level, commands, events):
        self.hero = hero
        self.level = level
        self.commands = commands
        self.events = events

class _LevelApi:
    pass

_level_api = _LevelApi()
_level_api.Hero = Hero
_level_api.Level = Level
_level_api.CommandLog = CommandLog
_level_api.Events = Events
_level_api.EvalContext = EvalContext

sys.modules["level_api"] = _level_api
"#,
    );
    prelude.push_str(&format!("_context_data = {context_literal}\n"));
    prelude.push_str(
        r#"hero = Hero(
    _context_data['hero']['x'],
    _context_data['hero']['y'],
    _context_data['hero']['steps'],
    _context_data['hero']['last_move'],
)
level = Level(
    _context_data['level']['width'],
    _context_data['level']['height'],
    (
        _context_data['level']['flag']['x'],
        _context_data['level']['flag']['y'],
    ),
    _context_data['level']['walls'],
)
commands = CommandLog(_context_data['commands'])
events = Events(
    _context_data['events']['reached_flag'],
    _context_data['events']['blocked_moves'],
    _context_data['events']['errors'],
    _context_data['events']['key_collected'],
    _context_data['events']['lock_unlocked'],
)
_context = EvalContext(hero, level, commands, events)
"#,
    );
    vm.run_code_string(scope.clone(), &prelude, "<eval_prelude>".to_string())
        .map_err(|err| format_python_error(vm, &err))?;
    vm.run_code_string(scope.clone(), code, "<evaluate>".to_string())
        .map_err(|err| format_python_error(vm, &err))?;
    vm.run_code_string(
        scope.clone(),
        "result = evaluate(_context)",
        "<evaluate_call>".to_string(),
    )
    .map_err(|err| format_python_error(vm, &err))?;

    let result_obj = scope
        .globals
        .get_item("result", vm)
        .map_err(|err| format_python_error(vm, &err))?;

    if let Ok(value) = result_obj.clone().try_into_value::<bool>(vm) {
        if value {
            return Ok(());
        }
        return Err("Evaluation failed".to_string());
    }
    if let Ok(value) = result_obj.try_into_value::<String>(vm) {
        return Err(value);
    }
    Err("evaluate() must return a bool or an error string".to_string())
}

fn format_python_error(vm: &VirtualMachine, err: &PyBaseExceptionRef) -> String {
    let mut buffer = String::new();
    if vm.write_exception(&mut buffer, err).is_ok() && !buffer.trim().is_empty() {
        return buffer;
    }
    format!("{err:?}")
}
