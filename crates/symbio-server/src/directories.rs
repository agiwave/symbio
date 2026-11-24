use serde::Serialize;
use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 4096;

#[derive(Debug, Serialize)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    pub kind: &'static str,
}

#[derive(Debug, Serialize)]
pub struct DirectoryListing {
    pub path: String,
    pub parent: Option<String>,
    pub roots: Vec<String>,
    pub entries: Vec<DirectoryEntry>,
    pub truncated: bool,
}

pub fn roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        ('A'..='Z')
            .map(|drive| PathBuf::from(format!("{drive}:\\")))
            .filter(|path| path.exists())
            .collect()
    }
    #[cfg(not(windows))]
    {
        vec![PathBuf::from("/")]
    }
}

pub fn list_directories(requested: Option<&str>) -> Result<DirectoryListing, String> {
    let roots = roots();
    let root_strings: Vec<String> = roots
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect();
    let Some(requested) = requested.filter(|value| !value.trim().is_empty()) else {
        return Ok(DirectoryListing {
            path: String::new(),
            parent: None,
            roots: root_strings,
            entries: roots
                .iter()
                .map(|path| DirectoryEntry {
                    name: path.to_string_lossy().to_string(),
                    path: path.to_string_lossy().to_string(),
                    kind: "directory",
                })
                .collect(),
            truncated: false,
        });
    };

    let expanded = if requested == "~" {
        dirs::home_dir().ok_or_else(|| "无法解析用户主目录".to_string())?
    } else if let Some(rest) = requested.strip_prefix("~/") {
        dirs::home_dir()
            .ok_or_else(|| "无法解析用户主目录".to_string())?
            .join(rest)
    } else {
        PathBuf::from(requested)
    };
    if !expanded.is_absolute() {
        return Err("目录路径必须是当前 Server 的绝对路径".to_string());
    }
    let path = expand_canonical(&expanded)?;

    let read_dir = std::fs::read_dir(&path)
        .map_err(|e| format!("读取目录失败 {path}: {e}", path = path.display()))?;
    let mut entries = Vec::new();
    let mut truncated = false;
    for entry in read_dir {
        let entry = entry.map_err(|e| format!("读取目录项失败: {e}"))?;
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }
        if entries.len() == MAX_ENTRIES {
            truncated = true;
            break;
        }
        entries.push(DirectoryEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            path: entry_path.to_string_lossy().to_string(),
            kind: "directory",
        });
    }
    entries.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    Ok(DirectoryListing {
        parent: path
            .parent()
            .map(|parent| parent.to_string_lossy().to_string()),
        path: path.to_string_lossy().to_string(),
        roots: root_strings,
        entries,
        truncated,
    })
}

fn expand_canonical(path: &Path) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|e| format!("目录不存在或不可访问 {}: {e}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!("路径不是目录: {}", canonical.display()));
    }
    Ok(canonical)
}

#[cfg(test)]
#[path = "directories.test.rs"]
mod tests;
