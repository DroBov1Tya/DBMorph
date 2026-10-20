use std::path::{Component, Path, PathBuf};

use anyhow::{Result, bail};

pub fn ensure_distinct_paths(input: &Path, output: &Path) -> Result<()> {
    if comparable_path(input)? == comparable_path(output)? {
        bail!("output {output:?} is the input file; choose a different output path");
    }
    Ok(())
}

fn comparable_path(path: &Path) -> Result<PathBuf> {
    if let Ok(canonical) = path.canonicalize() {
        return Ok(canonical);
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let parent = absolute.parent().unwrap_or(Path::new("/"));
    let name = absolute.file_name().unwrap_or_default();
    let resolved_parent = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());
    let mut normalized = PathBuf::new();
    for component in resolved_parent.join(name).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_relative_and_existing_paths() {
        let current = std::env::current_dir().unwrap();
        let path = current.join("Cargo.toml");
        assert!(ensure_distinct_paths(&path, Path::new("./Cargo.toml")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn matches_existing_symlink_alias() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("dbm-path-alias-{unique}"));
        std::fs::create_dir(&dir).unwrap();
        let input = dir.join("input.csv");
        let alias = dir.join("alias.csv");
        std::fs::write(&input, "data").unwrap();
        std::os::unix::fs::symlink(&input, &alias).unwrap();

        assert!(ensure_distinct_paths(&input, &alias).is_err());

        std::fs::remove_file(alias).unwrap();
        std::fs::remove_file(input).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
