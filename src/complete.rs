//! Directory completion for the workspace path field.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const LIMIT: usize = 4;

pub fn expand_user(input: &str) -> PathBuf {
    if input == "~" {
        return home_dir();
    }
    if let Some(rest) = input.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    if input.is_empty() {
        return home_dir();
    }
    PathBuf::from(input)
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn directory_history() -> Vec<String> {
    let Ok(output) = Command::new("zoxide").args(["query", "--list"]).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|path| Path::new(path).is_dir())
        .map(str::to_owned)
        .collect()
}

pub fn project_directories() -> Vec<String> {
    let home = home_dir();
    scan_project_roots(&[home.join("projects"), home.join("devdash")])
}

fn scan_project_roots(roots: &[PathBuf]) -> Vec<String> {
    let roots: Vec<_> = roots.iter().filter(|root| root.is_dir()).collect();
    if roots.is_empty() {
        return Vec::new();
    }
    let Ok(output) = Command::new("fd")
        .args([
            "--type",
            "d",
            "--max-depth",
            "4",
            "--print0",
            "--exclude",
            ".git",
            "--exclude",
            "node_modules",
            "--exclude",
            "target",
            "--exclude",
            ".venv",
            ".",
        ])
        .args(roots)
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let mut paths: Vec<_> = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .filter_map(|part| std::str::from_utf8(part).ok())
        .map(|path| path.trim_end_matches('/').to_owned())
        .collect();
    paths.sort_by_cached_key(|path| path.to_lowercase());
    paths
}

pub fn suggestions(input: &str, history: &[String], projects: &[String]) -> Vec<String> {
    let (parent, fragment, display_parent) = split_input(input);
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    let mut history_paths = Vec::new();

    for path in history {
        let expanded = expand_user(path);
        if expanded.is_dir() && seen.insert(expanded) {
            let display = accept(path);
            candidates.push(display.clone());
            history_paths.push(display);
        }
    }
    if let Ok(entries) = fs::read_dir(parent) {
        let mut children = Vec::new();
        for entry in entries.flatten() {
            if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                continue;
            }
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if name.starts_with('.') && !fragment.starts_with('.') {
                continue;
            }
            if seen.insert(entry.path()) {
                children.push((name.clone(), join_display(&display_parent, &name)));
            }
        }
        children.sort_by_cached_key(|entry| entry.0.to_lowercase());
        candidates.extend(children.into_iter().map(|(_, path)| path));
    }
    if !fragment.is_empty() {
        for path in projects {
            if seen.insert(PathBuf::from(path)) {
                candidates.push(accept(path));
            }
        }
    }
    if fragment.is_empty() {
        candidates.truncate(LIMIT);
        return candidates;
    }
    let mut results = fzf_matches(&fragment, &candidates, true);
    history_paths.retain(|path| !results.contains(path));
    results.extend(fzf_matches(&fragment, &history_paths, false));
    results
}

fn fzf_matches(fragment: &str, candidates: &[String], basename: bool) -> Vec<String> {
    if candidates.is_empty() {
        return Vec::new();
    }
    let mut command = Command::new("fzf");
    command.args([
        "--read0",
        "--print0",
        "--ignore-case",
        "--no-extended",
        "--tiebreak=index",
        "--filter",
        fragment,
    ]);
    if basename {
        // Paths keep their separators so tabs cannot corrupt the selected path.
        command.args(["--delimiter=/", "--nth=-2"]);
    }
    let Ok(mut child) = command
        .env_remove("FZF_DEFAULT_OPTS")
        .env_remove("FZF_DEFAULT_OPTS_FILE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    let stdin = child.stdin.take().unwrap();
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || -> std::io::Result<()> {
            let mut writer = std::io::BufWriter::new(stdin);
            for path in candidates {
                writer.write_all(path.as_bytes())?;
                writer.write_all(&[0])?;
            }
            writer.flush()
        });
        let output = child.wait_with_output();
        let wrote = matches!(writer.join(), Ok(Ok(())));
        let Ok(output) = output else {
            return Vec::new();
        };
        if !wrote || !output.status.success() {
            return Vec::new();
        }
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .filter_map(|part| std::str::from_utf8(part).ok().map(str::to_owned))
            .collect()
    })
}

pub fn accept(suggestion: &str) -> String {
    if suggestion.ends_with('/') {
        suggestion.to_owned()
    } else {
        format!("{suggestion}/")
    }
}

pub fn is_existing_dir(input: &str) -> bool {
    expand_user(input.trim()).is_dir()
}

pub fn default_name_from_dir(input: &str) -> String {
    expand_user(input.trim().trim_end_matches('/'))
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_owned()
}

pub fn browse_root(input: &str) -> PathBuf {
    let base = base_dir();
    let base = if base.is_absolute() {
        base
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| home_dir())
            .join(base)
    };
    if !input.trim().is_empty() {
        let path = expand_user(input.trim());
        let path = if path.is_absolute() {
            path
        } else {
            base.join(path)
        };
        if path.is_dir() {
            return path;
        }
        if let Some(parent) = path.parent().filter(|parent| parent.is_dir()) {
            return parent.to_path_buf();
        }
    }
    base
}

fn base_dir() -> PathBuf {
    ["WORKSPACE_FORM_CWD", "HERDR_WORKSPACE_CWD"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .find(|path| !path.is_empty())
        .map(|path| expand_user(&path))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| home_dir()))
}

fn split_input(input: &str) -> (PathBuf, String, String) {
    if input.is_empty() {
        let parent = base_dir();
        let display = format!("{}/", parent.display());
        return (parent, String::new(), display);
    }
    if input == "~" || input == "~/" {
        return (home_dir(), String::new(), String::from("~/"));
    }
    if input.ends_with('/') {
        return (expand_user(input), String::new(), input.to_owned());
    }
    let (display_parent, fragment) = match input.rsplit_once('/') {
        Some((parent, name)) => {
            let display = if parent.is_empty() {
                "/".to_owned()
            } else {
                format!("{parent}/")
            };
            (display, name.to_owned())
        }
        None => {
            let parent = base_dir();
            let display = format!("{}/", parent.display());
            return (parent, input.to_owned(), display);
        }
    };
    (expand_user(&display_parent), fragment, display_parent)
}

fn join_display(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        format!("{name}/")
    } else if parent.ends_with('/') {
        format!("{parent}{name}/")
    } else {
        format!("{parent}/{name}/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let tree = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        for name in ["alpha", "alpine", "beta", ".hidden"] {
            fs::create_dir(tree.path().join(name)).unwrap();
        }
        fs::write(tree.path().join("file.txt"), "file").unwrap();
        tree
    }

    #[test]
    fn completion_filters_child_names_and_excludes_files() {
        let tree = tree();
        let root = format!("{}/", tree.path().display());
        let children = suggestions(&root, &[], &[]);
        assert!(children.contains(&format!("{root}alpha/")));
        assert!(children.contains(&format!("{root}beta/")));
        assert!(!children.iter().any(|path| path.contains("file.txt")));
        let matched = suggestions(&format!("{root}apn"), &[], &[]);
        assert!(matched.contains(&format!("{root}alpine/")));
        assert!(!matched.contains(&format!("{root}beta/")));
        assert_eq!(
            suggestions(&format!("{root}.hi"), &[], &[]),
            [format!("{root}.hidden/")]
        );
    }

    #[test]
    fn history_matches_outside_parent_and_deduplicates_child() {
        let tree = tree();
        let outside = tempfile::tempdir().unwrap();
        let remote = outside.path().join("alpine-remote");
        fs::create_dir(&remote).unwrap();
        let child = tree.path().join("alpine").to_str().unwrap().to_owned();
        let history = vec![remote.to_str().unwrap().to_owned(), child.clone()];
        let matched = suggestions(
            &format!("{}/alp", tree.path().display()),
            &history,
            &[child.clone()],
        );
        assert_eq!(
            matched
                .iter()
                .filter(|path| **path == accept(&child))
                .count(),
            1
        );
        assert!(matched.contains(&accept(&history[0])));
        assert_eq!(
            suggestions("/missing/parent/alp", &[child.clone()], &[]),
            [accept(&child)]
        );
    }

    #[test]
    fn project_scan_limits_depth_and_dependency_directories() {
        let tree = tree();
        let root = tree.path().join("projects");
        let project = root.join("line/web-server");
        let deep = root.join("a/b/c/d/web-server");
        let dependency = root.join("node_modules/web-server");
        for path in [&project, &deep, &dependency] {
            fs::create_dir_all(path).unwrap();
        }
        let paths = scan_project_roots(&[root]);
        assert!(paths.contains(&project.to_str().unwrap().to_owned()));
        assert!(!paths.contains(&deep.to_str().unwrap().to_owned()));
        assert!(!paths.contains(&dependency.to_str().unwrap().to_owned()));
        assert!(suggestions("web", &[], &paths).contains(&accept(project.to_str().unwrap())));
        assert!(!suggestions("", &[], &paths).contains(&accept(project.to_str().unwrap())));
    }

    #[test]
    fn matching_preserves_tabs_and_spaces() {
        let tree = tree();
        for name in ["name with spaces", "name\twith\ttabs"] {
            let path = tree.path().join(name);
            fs::create_dir(&path).unwrap();
            assert!(
                suggestions("nwt", &[], &[path.to_str().unwrap().to_owned()])
                    .contains(&accept(path.to_str().unwrap()))
            );
        }
    }

    #[test]
    fn browser_root_uses_valid_input_or_existing_parent() {
        let tree = tree();
        assert_eq!(browse_root(tree.path().to_str().unwrap()), tree.path());
        assert_eq!(
            browse_root(tree.path().join("missing").to_str().unwrap()),
            tree.path()
        );
        assert!(browse_root("/missing/parent/directory").is_absolute());
    }

    #[test]
    fn directory_name_and_tilde_expansion_remain_available() {
        let tree = tree();
        let path = tree.path().join("alpha");
        assert!(is_existing_dir(path.to_str().unwrap()));
        assert_eq!(
            default_name_from_dir(&accept(path.to_str().unwrap())),
            "alpha"
        );
        assert_eq!(expand_user("~"), home_dir());
        assert_eq!(expand_user("~/src"), home_dir().join("src"));
    }
}
