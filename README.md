# qtail — Quiet Tail with On-Demand Output

`qtail` is a small Rust CLI that reads from `stdin`, stays quiet most of the time, and only writes to `stderr` when:

- a line matches a filter pattern (disabled by default),
- you press `space` (dump the last `N` lines), or
- input ends (EOF) / you press `Ctrl-C` (final dump, then exit).

## Install

### Build release binary

```bash
cargo build --release
```

Binary path:

```bash
target/release/qtail
```

### Install globally (on PATH)

Option 1 (recommended):

```bash
cargo install --path .
```

Option 2 (manual copy):

```bash
cp target/release/qtail ~/.cargo/bin/
```

Ensure `~/.cargo/bin` is on your `PATH`.

## Usage

```bash
qtail [OPTIONS]
```

Options:

- `-p, --pattern <PATTERN>`: substring match (case-insensitive), default empty (disabled)
- `-n, --lines <N>`: ring buffer size, default `20`
- `-h, --help`: help text

Typical usage:

```bash
long_running_cli_tool 2>&1 | qtail
```

## Output Streams (important)

`qtail` intentionally writes all visible output (matches + dumps) to `stderr`, not `stdout`.

- `stdout`: always empty
- `stderr`: `[match]` lines and dump blocks

This keeps `stdout` clean, but if you want to pipe `qtail` output to another command, merge `stderr` into `stdout`:

```bash
long_running_cli_tool 2>&1 | qtail 2>&1 | tee qtail.log
```

Or filter downstream:

```bash
long_running_cli_tool 2>&1 | qtail 2>&1 | grep -i match
```

## Examples

Output lines that contain "error" (case-insensitive):

```bash
some_tool 2>&1 | qtail --pattern error
```

Keep only the last 50 lines in memory:

```bash
some_tool 2>&1 | qtail --lines 50
```

## Development

Task runner recipes:

```bash
just --list
```

Common checks:

```bash
just check
```
