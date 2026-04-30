use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn warp_byok_proxy_help_lists_expected_subcommands() {
    let mut cmd = Command::cargo_bin("warp-byok-proxy").unwrap();
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("run"))
        .stdout(predicate::str::contains("cert"))
        .stdout(predicate::str::contains("login"));
}

#[test]
fn warp_byok_proxy_version_prints_0_0_1() {
    let mut cmd = Command::cargo_bin("warp-byok-proxy").unwrap();
    cmd.arg("--version");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("0.0.1"));
}
