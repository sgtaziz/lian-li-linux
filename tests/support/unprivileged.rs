fn run_as_desktop_user(test: &str) -> bool {
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    use std::time::{Duration, Instant};
    if unsafe { libc::geteuid() } != 0 {
        return false;
    }
    let directory = tempfile::tempdir().unwrap();
    let binary = directory.path().join("private-tests");
    std::fs::copy(std::env::current_exe().unwrap(), &binary).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::chown(directory.path(), Some(65534), Some(65534)).unwrap();
    let mut command = std::process::Command::new(binary);
    command.args(["--exact", test]).env("TMPDIR", directory.path());
    // Drop inherited root credentials using only async-signal-safe syscalls before exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setgroups(0, std::ptr::null()) != 0
                || libc::setgid(65534) != 0
                || libc::setuid(65534) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = spawn_test_process(&mut command)
        .unwrap_or_else(|error| panic!("Cannot launch unprivileged test {test}: {error}"));
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "Unprivileged test failed: {test}");
            return true;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("Unprivileged test timed out: {test}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn spawn_test_process(command: &mut std::process::Command) -> std::io::Result<std::process::Child> {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match command.spawn() {
            // Parallel test subprocesses can briefly inherit the executable's copy handle.
            Err(error)
                if error.raw_os_error() == Some(libc::ETXTBSY) && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(20));
            }
            result => return result,
        }
    }
}

#[test]
fn copied_test_process_waits_for_writer_to_close() {
    let directory = tempfile::tempdir().unwrap();
    let binary = directory.path().join("true");
    // NixOS has no /bin/true, so take the first one on PATH. Its read-only store mode is copied too.
    let source = std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|directory| directory.join("true"))
        .find(|path| path.is_file())
        .unwrap_or_else(|| "/bin/true".into());
    std::fs::copy(source, &binary).unwrap();
    std::fs::set_permissions(
        &binary,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    let writer = std::fs::OpenOptions::new().write(true).open(&binary).unwrap();
    let mut command = std::process::Command::new(&binary);
    assert_eq!(command.spawn().unwrap_err().raw_os_error(), Some(libc::ETXTBSY));
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(writer);
    });
    let result = spawn_test_process(&mut command);
    release.join().unwrap();
    assert!(result.unwrap().wait().unwrap().success());
}
