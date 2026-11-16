# texzi

Windows native text and Markdown editor built with Rust and NSH.

## Status

TX-00 native window baseline; an editor has not yet been delivered or validated.

## Build a private Windows x64 candidate

Use a clean checkout on Windows x64, PowerShell 7, Git, Rustup and Visual
Studio 2022 C++ build tools with Windows SDK. The exact Rust 1.97.0 toolchain
and rustfmt are selected by rust-toolchain.toml.

```powershell
cargo fmt --all -- --check
cargo build --release --locked --target x86_64-pc-windows-msvc
./scripts/build-candidate.ps1
```

Both commands use .cargo/config.toml to statically link the MSVC CRT
(`-C target-feature=+crt-static`); no additional command-line flags are used.
The script repeats the same checks/build, records toolchain, MSVC/SDK, runner
image, commit and resolved features, then creates dist/texzi-tx00-win-x64.zip
and SHA256SUMS.txt. Run it once in a clean checkout (dist must not exist).
CI uses the identical script on candidate branch push or workflow_dispatch.
It uploads a private Actions artifact, never a Release. Failure evidence is
uploaded separately and is not a successful candidate. No component tests are
present yet. See [runtime checks](docs/tx00/RUN.md) and
[dependency decisions and design errata](docs/tx00/DECISIONS.md).

The logging helper captures native stdout/stderr directly into separate files,
records Process.ExitCode before throwing on failure, and includes these files
and combined build.log in the candidate. transcript.log is supplementary.
`./scripts/test-native-logging.ps1` tests success, nonzero native exit, both
streams, large output and a failing build entry point without producing a
candidate. CI preserves injected-failure evidence separately from the candidate.
SDK/MSVC inventory is marked installed/default; it does not prove actual linker
or SDK selection.

The candidate script now selects an absolute x64 MSVC linker and explicit
MSVC/SDK library paths, sets the build-process-only target linker/LIB/encoded
Rust flags (static CRT, `--print=link-args`, explicit `/LIBPATH:` arguments),
and requires the successful final application link command to contain those
paths. `effective-toolchain-config.json` records version/hash and configuration;
`final-app-link-command.txt` is the actual rustc-emitted invocation. These flags
are extra evidence settings of the script/CI; the plain cargo command above
does not claim equivalent tool-selection tracing. See
[controlled real startup failure](docs/tx00/STARTUP_FAILURE.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the review, testing and merge policy.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
Third-party components retain their own licenses and attribution notices.
