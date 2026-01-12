use std::io::{Read, Write};
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

fn run_with_vm(
    vm: &VirtualMachine,
    code: &str,
    commands: Arc<Mutex<Vec<Command>>>,
) -> Result<(), String> {
    let scope = vm.new_scope_with_builtins();

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
            let mut buffer = commands.lock().map_err(|_| {
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

    let prelude = r#"
import sys

class _Game:
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

_game = _Game()
_game.hero = _Hero(__record_move)

sys.modules["game"] = _game
"#;

    vm.run_code_string(scope.clone(), prelude, "<prelude>".to_string())
        .map_err(|err| format_python_error(vm, &err))?;
    vm.run_code_string(scope, code, "<user>".to_string())
        .map_err(|err| format_python_error(vm, &err))?;

    Ok(())
}

fn format_python_error(vm: &VirtualMachine, err: &PyBaseExceptionRef) -> String {
    let mut buffer = String::new();
    if vm.write_exception(&mut buffer, err).is_ok() && !buffer.trim().is_empty() {
        return buffer;
    }
    format!("{err:?}")
}
