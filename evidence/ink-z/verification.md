# Ink Z candidate verification — 2026-11-16

Scope: native NSH Windows title-bar/application/executable icon only.
Branch: `feat/ink-z-app-icon`. No push, PR, merge or release.
Candidate identity: commit containing this record; ICO SHA256
`37BE184A1DAAB79DDA729F02D50D0C3F7C6314BF8051ED024B8D6EEBAF531839`.

Environment: Windows build 26300, x64; rustc 1.97.0
(`2d8144b7880597b6e6d3dfd63a9a9efae3f533d3`), LLVM 22.1.6.
PowerShell 7; existing local toolchain activated with:

```powershell
$env:CARGO_HOME='D:/Work/texzi/.tx00-tools/cargo'
$env:RUSTUP_HOME='D:/Work/texzi/.tx00-tools/rustup'
$env:PATH="$env:CARGO_HOME/bin;$env:PATH"
cargo fmt --all -- --check
rustc --edition 2024 --emit metadata --out-dir build-evidence/ink-z crates/texzi-app/build.rs
git diff --check
cargo build --release --locked --offline --target x86_64-pc-windows-msvc
```

- Static: first three checks exit 0; logs `fmt.log` and
  `build-script-check.log` (empty means no diagnostics).
- Asset structure: PowerShell `BitConverter` parsed ICO directory and checked
  every offset/length against file bounds, exit 0; `ico-frames.json` records
  all 10 dimensions. Source and copied ICO hashes match. Source 256px PNG
  visually inspected: warm porcelain board, dark ink Z, copper lower tip.
- White-box integration: pinned NSH `224d9a7`,
  `nsh-platform-windows/src/window.rs:170` loads icon resource 1 into the
  native window class. `has_own_icon` also checks resource 1. The build script
  emits that group with the original frames. Existing `build-candidate.ps1`
  copies the linked EXE into the ZIP. These are code observations, not
  application display evidence.
- Build: exit 101; `build.log` preserves failure. MSVC `link.exe` is absent.
  Compilation fails in dependency build scripts before application linking.
  No toolchain installation or shared tool-directory changes performed.
- Component tests: NOT_RUN; no application behavior changes. Resource
  generator type-checks but emitted .res and final PE remain unverified.
- Real application runtime / visual verification: NOT_RUN because no linked
  candidate is available. No screenshot or runtime display claim.
- Human acceptance: NOT_RUN; no acceptance PASS conclusion.

Remaining: build with MSVC C++ tools and Windows SDK; inspect resulting PE
icon group/frames, run the exact hashed executable and packaged ZIP from a
different directory, and capture title-bar/taskbar/Alt+Tab/Explorer display
at 100%, 150%, 200% DPI. Compare light/dark desktop presentation. Existing
Explorer or pinned-shortcut icon caches may require a fresh test path.
No installer exists, so installer-specific branding is outside this candidate.

Rollback: revert this task's commit; no framework, lockfile or package-script
changes. Preserve this failed-build evidence when continuing verification.
