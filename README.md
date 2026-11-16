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

The script repeats the same checks/build, records toolchain, MSVC/SDK, runner
image, commit and resolved features, then creates dist/texzi-tx00-win-x64.zip
and SHA256SUMS.txt. Run it once in a clean checkout (dist must not exist).
CI uses the identical script on candidate branch push or workflow_dispatch.
It uploads a private Actions artifact, never a Release. Failure evidence is
uploaded separately and is not a successful candidate. No component tests are
present yet. See [runtime checks](docs/tx00/RUN.md) and
[dependency decisions and design errata](docs/tx00/DECISIONS.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the review, testing and merge policy.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
Third-party components retain their own licenses and attribution notices.
