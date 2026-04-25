use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use tracing::{debug, error, info};

pub struct PidLock {
    _file: File,
}

impl PidLock {
    pub fn acquire() -> Result<Self> {
        let candidates = candidate_paths();
        let mut last_err: Option<anyhow::Error> = None;
        for path in candidates {
            match try_lock(&path) {
                Ok(file) => {
                    info!("Acquired pidlock at {}", path.display());
                    return Ok(Self { _file: file });
                }
                Err(e) => {
                    debug!("pidlock candidate {} unavailable: {e}", path.display());
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no pidlock candidate paths writable")))
    }
}

fn candidate_paths() -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from("/run/lianli-daemon.pid"),
        PathBuf::from("/var/run/lianli-daemon.pid"),
    ];
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        paths.push(PathBuf::from(xdg).join("lianli-daemon.pid"));
    }
    paths
}

fn try_lock(path: &std::path::Path) -> Result<File> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;

    let fd = file.as_raw_fd();
    let rc = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        let errno = std::io::Error::last_os_error();
        if errno.raw_os_error() == Some(libc::EWOULDBLOCK) {
            let mut existing = String::new();
            let _ = file.seek(SeekFrom::Start(0));
            let _ = file.read_to_string(&mut existing);
            let pid = existing.trim();
            error!(
                "Another lianli-daemon already holds {} (pid={}). Refusing to start.",
                path.display(),
                if pid.is_empty() { "?" } else { pid }
            );
            std::process::exit(1);
        }
        anyhow::bail!("flock {} failed: {errno}", path.display());
    }

    file.seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    writeln!(file, "{}", std::process::id())?;
    file.sync_all().ok();
    Ok(file)
}
