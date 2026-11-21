//! Output into a pipe whose reader is gone (`tooob show NAME | head -1`, #154): a command that
//! prints and exits ends quietly, as `ls | head` does, and a running Host serves on.
mod support;

use std::os::fd::{FromRawFd, OwnedFd};
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};
use support::home::{Home, a_folder};

/// The write end of a pipe whose read end is already closed: its first write gets EPIPE.
fn closed_pipe() -> Stdio {
    let mut ends = [0; 2];
    assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0);
    unsafe { libc::close(ends[0]) };
    Stdio::from(unsafe { OwnedFd::from_raw_fd(ends[1]) })
}

fn into_closed_pipe(home: &Home, args: &[&str]) -> Output {
    let mut cmd = home.command(args);
    cmd.stdout(closed_pipe());
    cmd.output().unwrap()
}

/// A Host named photos with a log: started and stopped once.
fn a_host_with_a_log(home: &Home) {
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let mut running = home.start("photos");
    assert!(running.stop_with(libc::SIGTERM).success());
}

#[test]
fn a_command_whose_output_pipe_is_closed_ends_quietly() {
    let home = Home::new();
    a_host_with_a_log(&home);
    for args in [
        &["--version"][..],
        &["--help"],
        &["ls"],
        &["ls", "--json"],
        &["show", "photos"],
        &["logs", "photos"],
    ] {
        let out = into_closed_pipe(&home, args);
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(err, "", "tooob {args:?} said on stderr");
        assert_ne!(out.status.code(), Some(101), "tooob {args:?} panicked");
        assert_ne!(out.status.code(), Some(1), "tooob {args:?} failed");
    }
}

#[test]
fn a_host_serves_on_when_its_stdout_and_stderr_close() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let mut cmd = home.command(&["run", "photos"]);
    cmd.stdout(closed_pipe()).stderr(closed_pipe());
    let mut host = cmd.spawn().unwrap();
    home.until_logged("photos", "serving");

    // It printed "serving" on its closed stderr too, and an Owner command talks to it.
    home.ok(&["show", "photos"]);
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until {
        assert!(host.try_wait().unwrap().is_none(), "the Host ended");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(home.ok(&["ls"]).contains("running"), "{}", home.ok(&["ls"]));
    unsafe { libc::kill(host.id() as i32, libc::SIGTERM) };
    assert!(host.wait().unwrap().success(), "it stops as it always does");
    home.until_logged("photos", "stopped");
}
