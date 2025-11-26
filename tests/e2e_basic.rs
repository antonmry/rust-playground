use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

fn bin() -> Command {
    let path = assert_cmd::cargo::cargo_bin!("energy-run");
    Command::new(path)
}

#[test]
fn runs_command_and_outputs_json_without_energy_fields_when_disabled() {
    let mut cmd = bin();
    let assert = cmd
        .args(["--no-cpu", "--no-gpu", "--output", "json", "--", "true"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"duration_s\""));

    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["command"], Value::Array(vec!["true".into()]));
    assert!(json["duration_s"].as_f64().unwrap() >= 0.0);
    assert!(json.get("cpu_energy_j").is_none());
    assert!(json.get("gpu_energy_j").is_none());
}

#[test]
fn propagates_exit_code() {
    let mut cmd = bin();
    let assert = cmd
        .args(["--no-cpu", "--no-gpu", "--output", "json", "--", "false"])
        .assert()
        .failure();

    let output = String::from_utf8_lossy(&assert.get_output().stdout);
    let json: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["exit_code"], 1);
}
