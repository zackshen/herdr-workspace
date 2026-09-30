//! Live directory completion for the workspace path field.

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

/// Visited directories, ordered by zoxide's ranking. Unavailable history is optional.
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

/// Nearby development trees, indexed once per form opening; never walk all of HOME.
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
        .filter_map(|part| {
            std::str::from_utf8(part)
                .ok()
                .map(|path| path.trim_end_matches('/').to_owned())
        })
        .collect();
    paths.sort_by_key(|path| path.to_lowercase());
    paths
}

/// Directory paths to replace `input` when accepted, highest match first.
pub fn suggestions(input: &str, history: &[String], projects: &[String]) -> Vec<String> {
    let (parent, fragment, display_parent) = split_input(input);
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    let mut history_paths = Vec::new();

    for path in history {
        let expanded = expand_user(path);
        if !expanded.is_dir() {
            continue;
        }
        let name = expanded
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_owned();
        if seen.insert(expanded) {
            let display = accept(path);
            candidates.push((name, display.clone()));
            history_paths.push(display);
        }
    }

    if let Ok(entries) = fs::read_dir(&parent) {
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
        children.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        candidates.extend(children);
    }
    if !fragment.is_empty() {
        for path in projects {
            if seen.insert(PathBuf::from(path)) {
                let name = Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                candidates.push((name.to_owned(), accept(path)));
            }
        }
    }

    if fragment.is_empty() {
        return candidates
            .into_iter()
            .take(LIMIT)
            .map(|(_, path)| path)
            .collect();
    }

    let mut results = fzf_matches(&fragment, &candidates);
    let remaining: Vec<_> = history_paths
        .into_iter()
        .filter(|path| !results.contains(path))
        .map(|path| (path.clone(), path))
        .collect();
    results.extend(fzf_matches(&fragment, &remaining));
    results
}

/// Match basenames (or full history paths on the second pass) without a popup.
fn fzf_matches(fragment: &str, candidates: &[(String, String)]) -> Vec<String> {
    if candidates.is_empty() {
        return Vec::new();
    }
    let Ok(mut child) = Command::new("fzf")
        .args([
            "--read0",
            "--print0",
            "--ignore-case",
            "--no-extended",
            "--tiebreak=index",
            "--delimiter=\\t",
            "--nth=1",
            "--accept-nth=2",
            "--filter",
            fragment,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    if let Some(mut stdin) = child.stdin.take() {
        for (name, path) in candidates {
            if write!(stdin, "{name}\t{path}\0").is_err() {
                return Vec::new();
            }
        }
    }
    let Ok(output) = child.wait_with_output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .filter_map(|part| std::str::from_utf8(part).ok().map(str::to_owned))
        .collect()
}

/// Accept a suggestion into the input buffer (directory + trailing slash).
pub fn accept(suggestion: &str) -> String {
    if suggestion.ends_with('/') {
        suggestion.to_string()
    } else {
        format!("{suggestion}/")
    }
}

pub fn is_existing_dir(input: &str) -> bool {
    let path = expand_user(input.trim());
    path.is_dir()
}

pub fn default_name_from_dir(input: &str) -> String {
    let path = expand_user(input.trim().trim_end_matches('/'));
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
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
    let ends_slash = input.ends_with('/');
    if ends_slash {
        let expanded = expand_user(input);
        return (expanded, String::new(), input.to_string());
    }
    let (display_parent, prefix) = match input.rsplit_once('/') {
        Some((parent, name)) => {
            let display = if parent.is_empty() {
                "/".to_string()
            } else {
                format!("{parent}/")
            };
            (display, name.to_string())
        }
        None => {
            let parent = base_dir();
            let display = format!("{}/", parent.display());
            return (parent, input.to_string(), display);
        }
    };
    (expand_user(&display_parent), prefix, display_parent)
}

fn join_display(display_parent: &str, name: &str) -> String {
    if display_parent.is_empty() {
        format!("{name}/")
    } else if display_parent.ends_with('/') {
        format!("{display_parent}{name}/")
    } else {
        format!("{display_parent}/{name}/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("alpha")).unwrap();
        fs::create_dir(dir.path().join("alpine")).unwrap();
        fs::create_dir(dir.path().join("beta")).unwrap();
        fs::write(dir.path().join("file.txt"), b"x").unwrap();
        fs::create_dir(dir.path().join(".hidden")).unwrap();
        dir
    }

    #[test]
    fn lists_child_dirs_after_slash() {
        let tree = setup_tree();
        let input = format!("{}/", tree.path().display());
        let got = suggestions(&input, &[], &[]);
        assert!(got.iter().any(|s| s.ends_with("alpha/")), "{got:?}");
        assert!(got.iter().any(|s| s.ends_with("beta/")), "{got:?}");
        assert!(!got.iter().any(|s| s.contains("file.txt")), "{got:?}");
        assert!(!got.iter().any(|s| s.contains(".hidden")), "{got:?}");
    }

    #[test]
    fn filters_by_prefix() {
        let tree = setup_tree();
        let input = format!("{}/al", tree.path().display());
        let got = suggestions(&input, &[], &[]);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got[0].ends_with("alpha/"), "{got:?}");
        assert!(got[1].ends_with("alpine/"), "{got:?}");
    }

    #[test]
    fn fuzzy_child_matches_without_prefix() {
        let tree = setup_tree();
        let got = suggestions(&format!("{}/apn", tree.path().display()), &[], &[]);
        assert_eq!(got, vec![format!("{}/alpine/", tree.path().display())]);
    }

    #[test]
    fn bare_fragment_finds_visited_directory() {
        let tree = tempfile::tempdir().unwrap();
        let visited = tree.path().join("alpine-visited");
        fs::create_dir(&visited).unwrap();
        let path = visited.to_string_lossy().into_owned();
        assert!(suggestions("", &[path.clone()], &[]).contains(&accept(&path)));
        assert!(suggestions("alpn", &[path.clone()], &[]).contains(&accept(&path)));
    }

    #[test]
    fn basename_ranking_precedes_history_order_and_full_path() {
        let tree = setup_tree();
        let outside = tempfile::tempdir().unwrap();
        let prefix = outside.path().join("apn-project");
        fs::create_dir(&prefix).unwrap();
        let history = vec![
            tree.path().join("alpine").to_string_lossy().into_owned(),
            prefix.to_string_lossy().into_owned(),
        ];
        let got = suggestions(&format!("{}/apn", tree.path().display()), &history, &[]);
        assert_eq!(got[0], accept(&history[1]));
        assert_eq!(got[1], accept(&history[0]));
    }

    #[test]
    fn history_matches_outside_parent_and_deduplicates_child() {
        let tree = setup_tree();
        let outside = tempfile::tempdir().unwrap();
        let visited = outside.path().join("alpine-remote");
        fs::create_dir(&visited).unwrap();
        let child = tree.path().join("alpine").to_string_lossy().into_owned();
        let history = vec![visited.to_string_lossy().into_owned(), child.clone()];
        let got = suggestions(&format!("{}/alp", tree.path().display()), &history, &[]);
        assert_eq!(
            got.iter().filter(|path| *path == &accept(&child)).count(),
            1
        );
        assert!(got.contains(&accept(&history[0])), "{got:?}");
    }

    #[test]
    fn missing_parent_still_offers_matching_history() {
        let tree = setup_tree();
        let visited = tree.path().join("alpine").to_string_lossy().into_owned();
        let got = suggestions("/no/such/parent/alp", &[visited.clone()], &[]);
        assert_eq!(got, vec![accept(&visited)]);
    }

    #[test]
    fn hidden_children_require_dot_fragment() {
        let tree = setup_tree();
        let root = format!("{}/", tree.path().display());
        assert!(!suggestions(&root, &[], &[])
            .iter()
            .any(|s| s.ends_with(".hidden/")));
        assert_eq!(
            suggestions(&format!("{root}.hi"), &[], &[]),
            vec![format!("{root}.hidden/")]
        );
    }

    #[test]
    fn empty_fragment_keeps_history_order_ahead_of_children() {
        let tree = setup_tree();
        let history = vec![
            tree.path().join("beta").to_string_lossy().into_owned(),
            tree.path().join("alpha").to_string_lossy().into_owned(),
        ];
        let got = suggestions(&format!("{}/", tree.path().display()), &history, &[]);
        assert_eq!(&got[..2], &[accept(&history[0]), accept(&history[1])]);
        assert!(got.len() <= 4);
    }

    #[test]
    fn project_scan_discovers_unvisited_nested_directory() {
        let tree = tempfile::tempdir().unwrap();
        let root = tree.path().join("projects");
        let web = root.join("line/web-server");
        let excluded = root.join("node_modules/web-server");
        let too_deep = root.join("a/b/c/d/web-server");
        fs::create_dir_all(&web).unwrap();
        fs::create_dir_all(&excluded).unwrap();
        fs::create_dir_all(&too_deep).unwrap();
        for i in 0..6 {
            fs::create_dir_all(root.join(format!("line/web-{i}"))).unwrap();
        }
        let discovered = scan_project_roots(&[root]);
        let path = web.to_string_lossy().into_owned();
        assert!(discovered.contains(&path));
        assert!(!discovered.contains(&excluded.to_string_lossy().into_owned()));
        assert!(!discovered.contains(&too_deep.to_string_lossy().into_owned()));
        let matches = suggestions("web", &[], &discovered);
        assert!(matches.len() > LIMIT);
        assert!(matches.contains(&accept(&path)));
        assert!(!suggestions("", &[], &discovered).contains(&accept(&path)));
    }

    #[test]
    fn project_history_and_child_candidates_are_deduplicated() {
        let tree = setup_tree();
        let path = tree.path().join("alpha").to_string_lossy().into_owned();
        let got = suggestions(
            &format!("{}/alp", tree.path().display()),
            &[path.clone()],
            &[path.clone()],
        );
        assert_eq!(
            got.iter().filter(|entry| *entry == &accept(&path)).count(),
            1
        );
    }

    #[test]
    fn expand_tilde() {
        let home = home_dir();
        assert_eq!(expand_user("~"), home);
        assert_eq!(expand_user("~/src"), home.join("src"));
    }

    #[test]
    fn existing_dir_and_name() {
        let tree = setup_tree();
        let path = tree.path().join("alpha");
        assert!(is_existing_dir(&path.to_string_lossy()));
        assert_eq!(default_name_from_dir(&path.to_string_lossy()), "alpha");
        assert_eq!(
            default_name_from_dir(&format!("{}/", path.display())),
            "alpha"
        );
    }

    #[test]
    fn join_preserves_tilde_display() {
        assert_eq!(join_display("~/", "src"), "~/src/");
        assert_eq!(join_display("/tmp/", "x"), "/tmp/x/");
    }
}
