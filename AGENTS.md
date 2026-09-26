# AGENTS.md

## Overview

`mcp-switchboard` is a high-performance byte-level reverse proxy fronting one or more backend MCP
(Model Context Protocol) servers, with optional features like tool search, response compaction,
and backend logging.

For full architecture details, configuration options, and feature explanations, refer to
[README.md](README.md).

## Testing

### Writing tests

Implement unit tests in the `tests/` directory, unless you need to test implementation details /
private code. In this specific case, tests can be colocated with the backend code.

### Running tests

Instead of running tests, prefer a codebase check using `cargo check`:

```sh
cargo check # only checks main library
cargo check --tests # checks main library + test code
cargo check --all-targets # checks all targets
```

Running tests is more "expensive". Avoid running them too often, "just to make sure". Prefer
running tests after you have reached a clear, major milestone. Use `cargo nextest` instead of
`cargo test`, which can be verbose. The following command suppresses progress output and reports
failed tests only along with a summary:

```sh
cargo nextest run --show-progress none --status-level fail
```

## Using `git`

**You are *NOT* allowed to use `git` commands.** Commits are managed by the project owner.
