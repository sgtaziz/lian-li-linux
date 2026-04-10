use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct SystemFont {
    pub family: String,
    pub path: PathBuf,
}

static CACHED_FONTS: OnceLock<Vec<SystemFont>> = OnceLock::new();

pub fn cached_system_fonts() -> &'static [SystemFont] {
    CACHED_FONTS.get_or_init(list_system_fonts).as_slice()
}

pub const DEFAULT_FONT_LABEL: &str = "(Default)";

pub fn font_label_for_path(path: Option<&Path>) -> String {
    let Some(p) = path else {
        return DEFAULT_FONT_LABEL.to_string();
    };
    cached_system_fonts()
        .iter()
        .find(|f| f.path == *p)
        .map(|f| f.family.clone())
        .unwrap_or_else(|| {
            p.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| p.display().to_string())
        })
}

pub fn font_path_for_label(label: &str) -> Option<PathBuf> {
    if label.is_empty() || label == DEFAULT_FONT_LABEL {
        return None;
    }
    cached_system_fonts()
        .iter()
        .find(|f| f.family == label)
        .map(|f| f.path.clone())
}

pub fn list_system_fonts() -> Vec<SystemFont> {
    let Ok(out) = Command::new("fc-list")
        .arg("--format=%{family[0]}\t%{file}\n")
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut fonts: Vec<SystemFont> = stdout
        .lines()
        .filter_map(|line| {
            let (family, file) = line.split_once('\t')?;
            let family = family.trim();
            let file = file.trim();
            if family.is_empty() || file.is_empty() {
                return None;
            }
            let lower = file.to_ascii_lowercase();
            if !(lower.ends_with(".ttf") || lower.ends_with(".otf")) {
                return None;
            }
            Some(SystemFont {
                family: family.to_string(),
                path: PathBuf::from(file),
            })
        })
        .collect();
    fonts.sort_by(|a, b| a.family.to_lowercase().cmp(&b.family.to_lowercase()));
    fonts.dedup_by(|a, b| a.family == b.family);
    fonts
}

pub fn default_font_path() -> Option<PathBuf> {
    if let Ok(out) = Command::new("fc-match")
        .arg("--format=%{file}")
        .arg("sans-serif")
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(PathBuf::from(s));
            }
        }
    }
    for candidate in [
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/liberation-sans/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    ] {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

pub fn resolve_font_path(path: Option<&std::path::Path>) -> Option<PathBuf> {
    if let Some(p) = path {
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }
    default_font_path()
}
