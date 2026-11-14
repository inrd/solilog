# Solilog

A fast, minimal, offline issue tracker for one. Native desktop app built in Rust with [GPUI](https://gpui.rs), plus a CLI that works on the same data (handy for scripts and AI agents).

## Run

```sh
cargo run                       # desktop app
cargo run -- --help             # CLI
cargo run -- issue create "Fix the thing"
cargo run -- issue ls --open --json
cargo run -- issue set-status SOL-1 done
cargo run -- issue create "Crash on load" -l acme/reverb -l bug
cargo run -- issue ls -l acme             # also matches sublabels like acme/reverb
cargo run -- label ls
```

## Keyboard

Everything is a command. Press `cmd-k` to open the command palette. Key bindings are customizable in `~/.config/solilog/keymap.json` (run the "Workspace: Open Keymap" command); `solilog keymap defaults` prints the defaults.

## Contributing

See [AGENTS.md](AGENTS.md) for architecture and conventions.

## License

[MIT](LICENSE)
