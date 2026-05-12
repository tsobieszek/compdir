use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Output};

#[derive(Debug, Clone, Copy)]
struct RenderOptions {
    hyperlink: bool,
    color: bool,
}

#[derive(Debug)]
struct CliOptions {
    help: bool,
    render: RenderOptions,
    positionals: Vec<String>,
    max_depth: Option<usize>,
}

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
    let cli = parse_cli(env::args().skip(1))?;

    if cli.help {
        print_help();
        return Ok(());
    }

    if matches!(cli.max_depth, Some(0)) {
        return Err("-L depth must be at least 1".to_string());
    }

    let (left_raw, right_raw) = match cli.positionals.as_slice() {
        [right] => (".", right.as_str()),
        [left, right] => (left.as_str(), right.as_str()),
        _ => {
            print_help();
            return Err("expected 1 or 2 arguments".to_string());
        }
    };

    let (left, right) = normalize_pair(left_raw, right_raw)?;

    let left_dirs = collect_dirs(&left, cli.max_depth)?;
    let right_dirs = collect_dirs(&right, cli.max_depth)?;

    let report = render_report(&left_dirs, &right_dirs, cli.render);
    if !report.is_empty() {
        println!("{report}");
    }

    Ok(())
}

fn print_help() {
    println!(
        "Usage:\n  compdir [-H|--hyperlink] [-c|--color] [-L<d>] <right>\n  compdir [-H|--hyperlink] [-c|--color] [-L<d>] <left> <right>\n  compdir -h|--help\n\nOptions:\n  -H, --hyperlink  add OSC 8 hyperlinks to the full paths\n  -c, --color      colorize basename rows in blue\n  -L<d>            restrict comparison to depth d (e.g. -L1 for top-level only)\n\nEach argument is `path`, `host:path`, or `host:`.\nIf one side is `host:` and the other is a local `path`, the remote side uses the local path's absolute form.\nIf both sides are `host:`, each side uses that host's current directory."
    );
}

fn parse_cli(args: impl IntoIterator<Item = String>) -> Result<CliOptions, String> {
    let mut help = false;
    let mut hyperlink = false;
    let mut color = false;
    let mut max_depth: Option<usize> = None;
    let mut positionals = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => help = true,
            "-H" | "--hyperlink" => hyperlink = true,
            "-c" | "--color" => color = true,
            s if s.starts_with("-L") => {
                let n = s[2..]
                    .parse::<usize>()
                    .map_err(|_| format!("invalid depth argument `{s}`: expected -L<integer>"))?;
                max_depth = Some(n);
            }
            _ => positionals.push(arg),
        }
    }

    Ok(CliOptions {
        help,
        render: RenderOptions { hyperlink, color },
        positionals,
        max_depth,
    })
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

fn collect_dirs(
    target: &ResolvedTarget,
    max_depth: Option<usize>,
) -> Result<BTreeMap<String, String>, String> {
    match target {
        ResolvedTarget::Local(root) => collect_local_dirs(root, max_depth),
        ResolvedTarget::Remote { host, root } => collect_remote_dirs(host, root, max_depth),
    }
}

fn collect_local_dirs(
    root: &Path,
    max_depth: Option<usize>,
) -> Result<BTreeMap<String, String>, String> {
    let mut dirs = BTreeMap::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];

    while let Some((dir, depth)) = stack.pop() {
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
                let new_depth = depth + 1;
                if !rel.as_os_str().is_empty() {
                    let rel_path = path_to_string(rel);
                    let full_path = path_to_string(&path);
                    dirs.insert(rel_path, local_uri(&full_path));
                }
                if max_depth.map_or(true, |d| new_depth < d) {
                    stack.push((path, new_depth));
                }
            }
        }
    }

    Ok(dirs)
}

fn collect_remote_dirs(
    host: &str,
    root: &str,
    max_depth: Option<usize>,
) -> Result<BTreeMap<String, String>, String> {
    let maxdepth_arg = max_depth
        .map(|d| format!("-maxdepth {d} "))
        .unwrap_or_default();
    let remote_cmd = format!(
        "cd -- {} && find . -mindepth 1 {maxdepth_arg}-type d -print0",
        shell_quote(root)
    );
    let output = run_ssh(host, &remote_cmd)?;
    if !output.status.success() {
        return Err(format_ssh_failure(host, &remote_cmd, &output));
    }

    let mut dirs = BTreeMap::new();
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
            let rel_path = bytes_to_path_string(rel);
            let full_path = join_remote_path(root, &rel_path);
            dirs.insert(rel_path, remote_uri(host, &full_path));
        }
    }
    Ok(dirs)
}

fn render_report(
    left_dirs: &BTreeMap<String, String>,
    right_dirs: &BTreeMap<String, String>,
    options: RenderOptions,
) -> String {
    let mut groups: BTreeMap<String, Vec<(Side, String, String)>> = BTreeMap::new();

    for (rel_path, full_path) in left_dirs {
        if !right_dirs.contains_key(rel_path) {
            groups.entry(basename(rel_path)).or_default().push((
                Side::Left,
                rel_path.clone(),
                full_path.clone(),
            ));
        }
    }
    for (rel_path, full_path) in right_dirs {
        if !left_dirs.contains_key(rel_path) {
            groups.entry(basename(rel_path)).or_default().push((
                Side::Right,
                rel_path.clone(),
                full_path.clone(),
            ));
        }
    }

    let mut blocks = Vec::new();
    for (basename, mut entries) in groups {
        entries.sort_by(|a, b| a.cmp(b));
        let mut block = Vec::with_capacity(entries.len() + 1);
        block.push(format_basename(&basename, options.color));
        for (side, rel_path, full_path) in entries {
            let display_path = if options.hyperlink {
                osc8_link(&rel_path, &full_path)
            } else {
                rel_path
            };
            block.push(format!("{} {}", side.prefix(), display_path));
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
    let mut components = path.components();
    let Some(first) = components.next() else {
        return String::new();
    };

    let mut output = first.as_os_str().to_string_lossy().into_owned();
    for component in components {
        if !output.ends_with('/') {
            output.push('/');
        }
        output.push_str(&component.as_os_str().to_string_lossy());
    }
    output
}

fn local_uri(path: &str) -> String {
    format!("file://{}", encode_uri_path(path))
}

fn remote_uri(host: &str, path: &str) -> String {
    format!("ssh://{host}{}", encode_uri_path(path))
}

fn join_remote_path(root: &str, rel_path: &str) -> String {
    if root == "/" {
        format!("/{rel_path}")
    } else {
        format!("{}/{}", root.trim_end_matches('/'), rel_path)
    }
}

fn encode_uri_path(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());

    for ch in path.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' => encoded.push(ch),
            _ => {
                let mut buffer = [0u8; 4];
                for byte in ch.encode_utf8(&mut buffer).as_bytes() {
                    encoded.push('%');
                    encoded.push_str(&format!("{byte:02X}"));
                }
            }
        }
    }

    encoded
}

fn format_basename(name: &str, color: bool) -> String {
    if color {
        blue(name)
    } else {
        name.to_string()
    }
}

fn blue(text: &str) -> String {
    format!("\x1b[34m{text}\x1b[0m")
}

fn osc8_link(text: &str, uri: &str) -> String {
    format!("\x1b]8;;{uri}\x1b\\{text}\x1b]8;;\x1b\\")
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
        let left: BTreeMap<String, String> = [
            "a/b/my-dir".to_string(),
            "c/d/e/my-dir".to_string(),
            "mydir2".to_string(),
        ]
        .into_iter()
        .map(|rel| (rel.clone(), format!("file:///left/{rel}")))
        .collect();
        let right: BTreeMap<String, String> = [
            "a/b/my-dir".to_string(),
            "x/my-dir".to_string(),
            "z/w/s/my-dir".to_string(),
        ]
        .into_iter()
        .map(|rel| (rel.clone(), format!("file:///right/{rel}")))
        .collect();

        let rendered = render_report(
            &left,
            &right,
            RenderOptions {
                hyperlink: false,
                color: false,
            },
        );
        assert_eq!(
            rendered,
            "my-dir\nL c/d/e/my-dir\nR x/my-dir\nR z/w/s/my-dir\n\nmydir2\nL mydir2"
        );
    }

    #[test]
    fn renders_hyperlinks_and_color() {
        let left: BTreeMap<String, String> =
            [("my-dir".to_string(), "file:///left/my-dir".to_string())]
                .into_iter()
                .collect();
        let right: BTreeMap<String, String> = BTreeMap::new();

        let rendered = render_report(
            &left,
            &right,
            RenderOptions {
                hyperlink: true,
                color: true,
            },
        );

        assert_eq!(
            rendered,
            "\x1b[34mmy-dir\x1b[0m\nL \x1b]8;;file:///left/my-dir\x1b\\my-dir\x1b]8;;\x1b\\"
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

    #[test]
    fn path_to_string_preserves_absolute_roots() {
        assert_eq!(path_to_string(Path::new("/a/b")), "/a/b");
    }

    #[test]
    fn uri_helpers_encode_spaces() {
        assert_eq!(local_uri("/tmp/a b"), "file:///tmp/a%20b");
        assert_eq!(remote_uri("host", "/tmp/a b"), "ssh://host/tmp/a%20b");
    }

    #[test]
    fn cli_flags_can_be_combined_with_positionals() {
        let cli = parse_cli([
            "-H".to_string(),
            "left".to_string(),
            "--color".to_string(),
            "right".to_string(),
        ])
        .unwrap();

        assert!(!cli.help);
        assert!(cli.render.hyperlink);
        assert!(cli.render.color);
        assert_eq!(cli.positionals, vec!["left", "right"]);
        assert_eq!(cli.max_depth, None);
    }

    #[test]
    fn cli_parses_depth_flag() {
        let cli = parse_cli(["-L3".to_string(), "right".to_string()]).unwrap();
        assert_eq!(cli.max_depth, Some(3));
        assert_eq!(cli.positionals, vec!["right"]);
    }

    #[test]
    fn cli_rejects_invalid_depth_flag() {
        assert!(parse_cli(["-Lfoo".to_string()]).is_err());
    }
}
