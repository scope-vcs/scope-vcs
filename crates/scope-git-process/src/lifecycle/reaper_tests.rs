use super::*;
use std::{
    io::{BufRead, BufReader},
    process::Stdio,
};

// Signal handlers and waitpid(-1) are process-wide. Run each scenario in its
// own test process so it cannot handle signals or reap children for other tests.
fn isolated(name: &str, exit_code: i32, scenario: impl FnOnce()) {
    let test_name = format!("lifecycle::reaper_tests::{name}");
    if std::env::var("SCOPE_REAPER_TEST").as_deref() == Ok(test_name.as_str()) {
        scenario();
        return;
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", &test_name, "--nocapture", "--test-threads=1"])
        .env("SCOPE_REAPER_TEST", &test_name);
    let started = Instant::now();
    let output = crate::run(
        &mut command,
        None,
        crate::ProcessLimits::new(Duration::from_secs(5)),
        "isolated reaper regression",
    )
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(exit_code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "signal forwarding must not wait for the service's fallback exit"
    );
}

fn service_waiting_for_term() -> ChildGuard {
    let mut command = Command::new("sh");
    // Start the fallback before announcing readiness. A foreground sleep started
    // afterward could miss the group signal and delay the shell's TERM trap.
    command
        .args([
            "-c",
            "trap 'exit 23' TERM; sleep 3 & printf 'ready\\n'; wait; exit 24",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_process_group(&mut command);
    let mut child = ChildGuard::new(command.spawn().unwrap());
    let mut ready = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready, "ready\n");
    child
}

#[test]
fn signal_between_pending_check_and_wait_remains_responsive() {
    isolated(
        "signal_between_pending_check_and_wait_remains_responsive",
        0,
        || {
            install_reaper_signal_handlers().unwrap();
            let mut service = service_waiting_for_term();
            let service_pid = service.id() as i32;
            forward_pending_signal(service_pid);
            // SAFETY: this isolated process installed a handler for SIGTERM above.
            assert_eq!(unsafe { libc::raise(libc::SIGTERM) }, 0);
            assert_eq!(reap_exited_child().unwrap(), None);
            forward_pending_signal(service_pid);
            assert_eq!(service.wait().unwrap().code(), Some(23));
            service.disarm();
        },
    );
}

#[test]
fn reaper_forwards_a_signal_while_the_service_is_idle() {
    isolated(
        "reaper_forwards_a_signal_while_the_service_is_idle",
        23,
        || {
            install_reaper_signal_handlers().unwrap();
            let service = service_waiting_for_term();
            thread::spawn(|| {
                thread::sleep(Duration::from_millis(75));
                // SAFETY: signal only this isolated reaper test process.
                assert_eq!(
                    unsafe { libc::kill(std::process::id() as i32, libc::SIGTERM) },
                    0
                );
            });
            reap_service_process(service.id()).unwrap();
            unreachable!("the reaper exits with its service's status");
        },
    );
}

#[test]
fn reaper_preserves_a_normal_service_exit() {
    isolated("reaper_preserves_a_normal_service_exit", 7, || {
        install_reaper_signal_handlers().unwrap();
        let mut command = Command::new("sh");
        command.args(["-c", "exit 7"]);
        configure_process_group(&mut command);
        let service = ChildGuard::new(command.spawn().unwrap());
        reap_service_process(service.id()).unwrap();
        unreachable!("the reaper exits with its service's status");
    });
}
