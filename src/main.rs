use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Output};

#[derive(Debug, Clone)]
enum ParsedArg {
    Local(PathBuf),
    RemoteEmpty { host: String },
    RemotePath { host: String, path: String },
}

#[derive(Debug, Clone)]
enum ResolvedTarget {
    Local(PathBuf),
    Remote { host: String, root: String },
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum Side {
    Left,
    Right,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_help();
        return Ok(());
    }

    let (left_raw, right_raw) = match args.as_slice() {
        [right] => (".", right.as_str()),
        [left, right] => (left.as_str(), right.as_str()),
        _ => {
            print_help();
            return Err("expected 1 or 2 arguments".to_string());
        }
    };

    let (left, right) = normalize_pair(left_raw, right_raw)?;

    let left_dirs = collect_dirs(&left)?;
    let right_dirs = collect_dirs(&right)?;

    let left_only: BTreeSet<String> = left_dirs.difference(&right_dirs).cloned().collect();
    let right_only: BTreeSet<String> = right_dirs.difference(&left_dirs).cloned().collect();

    let report = render_report(&left_only, &right_only);
    if !report.is_empty() {
        println!("{report}");
    }

    Ok(())
}

fn print_help() {
    println!(
        "Usage:\n  compdir <right>\n  compdir <left> <right>\n\nEach argument is `path`, `host:path`, or `host:`.\nIf one side is `host:` and the other is a local `path`, the remote side uses the local path's absolute form.\nIf both sides are `host:`, each side uses that host's current directory."
    );
}

fn normalize_pair(
    left_raw: &str,
    right_raw: &str,
) -> Result<(ResolvedTarget, ResolvedTarget), String> {
    let left = parse_arg(left_raw)?;
    let right = parse_arg(right_raw)?;

    match (&left, &right) {
        (ParsedArg::RemoteEmpty { host }, ParsedArg::Local(local_path)) => {
            let absolute = resolve_local_path(local_path)?;
            Ok((
                ResolvedTarget::Remote {
                    host: host.clone(),
                    root: path_to_string(&absolute),
                },
                ResolvedTarget::Local(absolute),
            ))
        }
        (ParsedArg::Local(local_path), ParsedArg::RemoteEmpty { host }) => {
            let absolute = resolve_local_path(local_path)?;
            Ok((
                ResolvedTarget::Local(absolute.clone()),
                ResolvedTarget::Remote {
                    host: host.clone(),
                    root: path_to_string(&absolute),
                },
            ))
        }
        _ => Ok((resolve_target(left)?, resolve_target(right)?)),
    }
}

fn parse_arg(raw: &str) -> Result<ParsedArg, String> {
    if let Some((host, path)) = raw.split_once(':') {
        if host.is_empty() {
            return Err(format!("invalid argument `{raw}`: host is empty"));
        }
        if path.is_empty() {
            Ok(ParsedArg::RemoteEmpty {
                host: host.to_string(),
            })
        } else {
            Ok(ParsedArg::RemotePath {
                host: host.to_string(),
                path: path.to_string(),
            })
        }
    } else {
        Ok(ParsedArg::Local(PathBuf::from(raw)))
    }
}

fn resolve_target(arg: ParsedArg) -> Result<ResolvedTarget, String> {
    match arg {
        ParsedArg::Local(path) => Ok(ResolvedTarget::Local(resolve_local_path(&path)?)),
        ParsedArg::RemoteEmpty { host } => Ok(ResolvedTarget::Remote {
            host: host.clone(),
            root: resolve_remote_path(&host, None)?,
        }),
        ParsedArg::RemotePath { host, path } => Ok(ResolvedTarget::Remote {
            host: host.clone(),
            root: resolve_remote_path(&host, Some(&path))?,
        }),
    }
}

fn resolve_local_path(path: &Path) -> Result<PathBuf, String> {
    let absolute = fs::canonicalize(path)
        .map_err(|err| format!("failed to resolve local path `{}`: {err}", path.display()))?;
    if !absolute.is_dir() {
        return Err(format!("`{}` is not a directory", path.display()));
    }
    Ok(absolute)
}

fn resolve_remote_path(host: &str, path: Option<&str>) -> Result<String, String> {
    let remote_cmd = match path {
        Some(path) => format!("cd -- {} && pwd -P", shell_quote(path)),
        None => "pwd -P".to_string(),
    };
    let output = run_ssh(host, &remote_cmd)?;
    if !output.status.success() {
        return Err(format_ssh_failure(host, &remote_cmd, &output));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| format!("`ssh {host}` returned a non-UTF-8 path"))?;
    let root = stdout.trim();
    if root.is_empty() {
        return Err(format!("`ssh {host}` returned an empty path"));
    }
    Ok(root.to_string())
}

fn collect_dirs(target: &ResolvedTarget) -> Result<BTreeSet<String>, String> {
    match target {
        ResolvedTarget::Local(root) => collect_local_dirs(root),
        ResolvedTarget::Remote { host, root } => collect_remote_dirs(host, root),
    }
}

fn collect_local_dirs(root: &Path) -> Result<BTreeSet<String>, String> {
    let mut dirs = BTreeSet::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir)
            .map_err(|err| format!("failed to read directory `{}`: {err}", dir.display()))?;

        for entry in entries {
            let entry = entry
                .map_err(|err| format!("failed to read entry in `{}`: {err}", dir.display()))?;
            let file_type = entry
                .file_type()
                .map_err(|err| format!("failed to inspect `{}`: {err}", entry.path().display()))?;
            if file_type.is_dir() {
                let path = entry.path();
                let rel = path.strip_prefix(root).map_err(|_| {
                    format!("failed to compute relative path for `{}`", path.display())
                })?;
                if !rel.as_os_str().is_empty() {
                    dirs.insert(path_to_string(rel));
                }
                stack.push(path);
            }
        }
    }

    Ok(dirs)
}

fn collect_remote_dirs(host: &str, root: &str) -> Result<BTreeSet<String>, String> {
    let remote_cmd = format!(
        "cd -- {} && find . -mindepth 1 -type d -print0",
        shell_quote(root)
    );
    let output = run_ssh(host, &remote_cmd)?;
    if !output.status.success() {
        return Err(format_ssh_failure(host, &remote_cmd, &output));
    }

    let mut dirs = BTreeSet::new();
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let rel = if raw.starts_with(b"./") {
            &raw[2..]
        } else {
            raw
        };
        if !rel.is_empty() {
            dirs.insert(bytes_to_path_string(rel));
        }
    }
    Ok(dirs)
}

fn render_report(left_only: &BTreeSet<String>, right_only: &BTreeSet<String>) -> String {
    let mut groups: BTreeMap<String, Vec<(Side, String)>> = BTreeMap::new();

    for rel_path in left_only {
        groups
            .entry(basename(rel_path))
            .or_default()
            .push((Side::Left, rel_path.clone()));
    }
    for rel_path in right_only {
        groups
            .entry(basename(rel_path))
            .or_default()
            .push((Side::Right, rel_path.clone()));
    }

    let mut blocks = Vec::new();
    for (basename, mut entries) in groups {
        entries.sort_by(|a, b| a.cmp(b));
        let mut block = Vec::with_capacity(entries.len() + 1);
        block.push(basename);
        for (side, rel_path) in entries {
            block.push(format!("{} {}", side.prefix(), rel_path));
        }
        blocks.push(block.join("\n"));
    }

    blocks.join("\n\n")
}

fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .unwrap_or_else(|| OsStr::new(path))
        .to_string_lossy()
        .into_owned()
}

fn path_to_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn bytes_to_path_string(path: &[u8]) -> String {
    let text = String::from_utf8_lossy(path);
    text.trim_start_matches("./").to_string()
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }

    let mut quoted = String::from("'");
    for ch in value.chars() {
        if ch == '\'' {
            quoted.push_str("'\"'\"'");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

fn run_ssh(host: &str, remote_cmd: &str) -> Result<Output, String> {
    Command::new("ssh")
        .arg(host)
        .arg(remote_cmd)
        .output()
        .map_err(|err| format!("failed to run `ssh {host}`: {err}"))
}

fn format_ssh_failure(host: &str, remote_cmd: &str, output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!(
        "`ssh {host}` failed while running `{remote_cmd}`\n{}",
        stderr.trim()
    )
}

impl Side {
    fn prefix(self) -> &'static str {
        match self {
            Side::Left => "L",
            Side::Right => "R",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_expected_grouping() {
        let left: BTreeSet<String> = [
            "a/b/my-dir".to_string(),
            "c/d/e/my-dir".to_string(),
            "mydir2".to_string(),
        ]
        .into_iter()
        .collect();
        let right: BTreeSet<String> = [
            "a/b/my-dir".to_string(),
            "x/my-dir".to_string(),
            "z/w/s/my-dir".to_string(),
        ]
        .into_iter()
        .collect();

        let left_only: BTreeSet<String> = left.difference(&right).cloned().collect();
        let right_only: BTreeSet<String> = right.difference(&left).cloned().collect();

        let rendered = render_report(&left_only, &right_only);
        assert_eq!(
            rendered,
            "my-dir\nL c/d/e/my-dir\nR x/my-dir\nR z/w/s/my-dir\n\nmydir2\nL mydir2"
        );
    }

    #[test]
    fn quotes_single_quotes_for_shell() {
        assert_eq!(shell_quote("abc"), "'abc'");
        assert_eq!(shell_quote("a'b"), "'a'\"'\"'b'");
    }

    #[test]
    fn basename_uses_last_component() {
        assert_eq!(basename("a/b/c"), "c");
    }
}
