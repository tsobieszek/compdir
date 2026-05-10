# compdir

`compdir` compares directory trees and prints the relative directories that are present on only one side.

## Usage

```text
compdir <right>
compdir <left> <right>
compdir -h
compdir --help
```

With one positional argument, `compdir` uses the current directory as the left side and the provided argument as the right side.

## Argument Forms

Each positional argument can be one of the following:

- `path`
- `host:path`
- `host:`

Resolution rules:

- Local `path` arguments are resolved to absolute paths before scanning.
- `host:path` is scanned over SSH from the resolved remote path.
- `host:` uses the remote host's current directory.
- If one side is `host:` and the other side is a local `path`, the local path is resolved first and then reused as the remote root.
- If both sides are `host:`, each side uses its own host's current directory.

## What It Compares

`compdir` recursively scans both sides and keeps directories only.

It then:

- converts each directory to a path relative to its comparison root
- removes relative paths that appear on both sides
- groups the remaining paths by basename
- sorts paths lexicographically within each basename group
- prints blank-line-separated blocks

## Output Format

Each block starts with the basename and then lists the unique relative paths prefixed with `L` or `R`:

```text
my-dir
L c/d/e/my-dir
R x/my-dir
R z/w/s/my-dir

mydir2
L mydir2
```

`L` refers to the left argument and `R` refers to the right argument.

## Implementation Notes

- CLI flow and help: [`src/main.rs`](src/main.rs)
- Argument parsing and target normalization: [`src/main.rs`](src/main.rs)
- Local and SSH directory scanning: [`src/main.rs`](src/main.rs)
- Report rendering: [`src/main.rs`](src/main.rs)

## Validation

- `cargo test`
- `cargo build`
