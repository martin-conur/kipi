use assert_cmd::Command;
use predicates::prelude::*;

fn kipi() -> Command {
    Command::cargo_bin("kipi").expect("kipi binary is built")
}

#[test]
fn version_prints_crate_version() {
    kipi()
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("kipi {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_prints_usage() {
    kipi()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: kipi"))
        .stdout(predicate::str::contains("--version"));
}

#[test]
fn unknown_flag_is_a_usage_error() {
    kipi().arg("--no-such-flag").assert().failure();
}
