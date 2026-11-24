//! Installed font discovery. On Windows the font registry is used
//! (family name -> file); elsewhere the usual font directories are scanned.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct FontEntry {
    pub name: String,
    pub path: PathBuf,
}

fn supported(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("ttf" | "otf" | "ttc")
    )
}

/// All installed fonts usable by the overlay, sorted by name.
pub fn list_fonts() -> Vec<FontEntry> {
    let mut v = platform::list();
    v.retain(|f| supported(&f.path));
    v.sort_by_cached_key(|f| f.name.to_lowercase());
    v.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    v
}

/// Resolve a family name or a file path to a font file.
pub fn resolve(name_or_path: &str) -> Option<PathBuf> {
    let p = Path::new(name_or_path);
    if p.is_file() {
        return Some(p.to_path_buf());
    }
    if let Some(f) = list_fonts()
        .into_iter()
        .find(|f| f.name.eq_ignore_ascii_case(name_or_path.trim()))
    {
        return Some(f.path);
    }
    {
        // fontconfig always answers with *something*; only accept it for
        // generic names, otherwise a typo would silently become DejaVu.
        let generic = ["sans", "serif", "monospace", "sans bold", "sans-serif"];
        if generic
            .iter()
            .any(|g| name_or_path.trim().eq_ignore_ascii_case(g))
        {
            return platform::fc_match(name_or_path);
        }
    }
    None
}

mod platform {
    use super::FontEntry;
    use std::path::{Path, PathBuf};

    /// fontconfig knows real family/style names; use it when present.
    fn fc_list() -> Option<Vec<FontEntry>> {
        let out = std::process::Command::new("fc-list")
            .args([":", "--format", "%{family[0]}\t%{style[0]}\t%{file}\n"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let v: Vec<FontEntry> = text
            .lines()
            .filter_map(|l| {
                let mut it = l.split('\t');
                let (fam, style, file) = (it.next()?, it.next()?, it.next()?);
                let name = if style.is_empty()
                    || style.eq_ignore_ascii_case("regular")
                    || style.eq_ignore_ascii_case("book")
                {
                    fam.to_string()
                } else {
                    format!("{fam} {style}")
                };
                Some(FontEntry {
                    name,
                    path: PathBuf::from(file),
                })
            })
            .collect();
        (!v.is_empty()).then_some(v)
    }

    /// Best match for any fontconfig pattern ("Sans Bold", "monospace").
    pub fn fc_match(name: &str) -> Option<PathBuf> {
        // "Noto Sans Bold" -> family "Noto Sans", style "Bold" is handled by fc.
        let out = std::process::Command::new("fc-match")
            .args(["--format", "%{file}", name])
            .output()
            .ok()?;
        let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        (out.status.success() && p.is_file()).then_some(p)
    }

    pub fn list() -> Vec<FontEntry> {
        if let Some(v) = fc_list() {
            return v;
        }
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let dirs = [
            PathBuf::from("/usr/share/fonts"),
            PathBuf::from("/usr/local/share/fonts"),
            home.join(".local/share/fonts"),
            home.join(".fonts"),
            PathBuf::from("/System/Library/Fonts"),
            PathBuf::from("/Library/Fonts"),
            home.join("Library/Fonts"),
        ];
        let mut out = Vec::new();
        for d in dirs {
            walk(&d, &mut out, 0);
        }
        out
    }

    fn walk(dir: &Path, out: &mut Vec<FontEntry>, depth: u32) {
        if depth > 6 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out, depth + 1);
            } else if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                out.push(FontEntry {
                    name: stem.to_string(),
                    path: p.clone(),
                });
            }
        }
    }
}
