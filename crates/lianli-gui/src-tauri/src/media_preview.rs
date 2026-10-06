use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

const MAX_PREVIEW_BYTES: u64 = 64 * 1024 * 1024;
const PREVIEW_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "bmp", "gif", "mp4", "webm", "mkv", "mov", "avi", "m4v",
];

pub fn read(path: &Path) -> Result<Vec<u8>, String> {
    if !path.is_absolute() {
        return Err("Preview needs an absolute media path".into());
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !PREVIEW_EXTENSIONS.contains(&extension.as_str()) {
        return Err("This file type cannot be previewed".into());
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| format!("Cannot open media: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Cannot inspect media: {error}"))?;
    if !metadata.is_file() {
        return Err("Preview needs a regular file".into());
    }
    if metadata.len() > MAX_PREVIEW_BYTES {
        return Err("Media file is too large to preview".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_PREVIEW_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read media: {error}"))?;
    if bytes.len() as u64 > MAX_PREVIEW_BYTES {
        return Err("Media file is too large to preview".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::read;
    use std::path::Path;

    #[test]
    fn reads_supported_media_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("clip.GIF");
        std::fs::write(&path, b"GIF89a").unwrap();
        assert_eq!(read(&path).unwrap(), b"GIF89a");
    }

    #[test]
    fn rejects_relative_unsupported_and_non_file_paths() {
        let directory = tempfile::tempdir().unwrap();
        let text = directory.path().join("notes.txt");
        std::fs::write(&text, b"text").unwrap();
        assert!(read(Path::new("clip.gif")).is_err());
        assert!(read(&text).is_err());
        let folder = directory.path().join("folder.png");
        std::fs::create_dir(&folder).unwrap();
        assert!(read(&folder).is_err());
    }

    #[test]
    fn fifo_paths_are_rejected_without_blocking() {
        let directory = tempfile::tempdir().unwrap();
        let fifo = directory.path().join("pipe.gif");
        let c_path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || sender.send(read(&fifo).is_err()).unwrap());
        assert!(receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("reading a FIFO blocked"));
    }
}
