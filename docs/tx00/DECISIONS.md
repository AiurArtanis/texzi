# TX-00 baseline and errata

Rusty confirmed the local TX-00 task sheet and HANDOFF T00/G0 as the technical
contract. The current task overrides old main-push instructions: candidate
branch only; no merge, tag, Release or publication.

The imported design-v1 directory remains byte-for-byte unchanged. Its 50
manifest entries were verified for length and SHA256 with no differences.
The source design and original untracked docs/evidence remain untouched.

Errata applied by this separate record (not by rewriting design originals):

- PERFORMANCE_PLAN.md is authoritative: each valid cold-start measurement
  must meet its limit; the p95 wording in DECISIONS.md does not override it.
- IME/UIA belong to G4 and save reliability to G3. Keep task/AC mappings even
  where historical Gate numbering differs.

NSH source inspected at 224d9a795c7ac9d10d75750de9351adac5549675:
root Cargo.toml declares 0.5.1, edition 2024, rust-version 1.97 and Apache-2.0;
rust-toolchain.toml uses floating stable upstream. texzi instead pins 1.97.0,
verified locally as rustc 2d8144b78 (2026-07-07), LLVM 22.1.6.
Windows backend Cargo.toml and src/app.rs, src/lib.rs, plus tasks-rs native
example were read. run_app uses Win32/Direct2D/DirectWrite and returns ExitCode;
failure paths log, show a native error dialog and return failure. Its standalone
loop owns native thread teardown. These are code observations, not runtime proof.

Both direct NSH dependencies disable default features; no optional NSH feature
is requested. Windows backend still transitively enables core svg/loader and
engine net. It unconditionally depends on WebView2 bindings, but this app creates
no WebView or JS bundle. Full resolved features are emitted by cargo tree in CI.
log is fixed at 0.4.29. Cargo.lock pins transitive dependencies; metadata.json
and sbom.json enumerate source/version/license. No NSH patch/fork is introduced.

Fonts are system-provided, none bundled. Future editor font policy is Cascadia
Mono/Consolas with Windows fallback; precise availability and font versions are
future checks, not a claim of shipped fonts. PCRE2/binding, parser, document
kernel and future UI dependencies remain TBD and are not added speculatively.
MSVC/SDK follow the windows-2022 image, with exact installed versions recorded
per run. Runner image is mutable; recorded image identity is required evidence.
SDK directory listings and the MSVC default-version file are explicitly recorded
as installed/default inventory, not the actual linker/SDK chosen by rustc.
tool-inventory.json carries this boundary into every candidate package. Effective
tool selection is documented separately when effective evidence exists.
TX-02 now selects the final linker by absolute path and MSVC/SDK library search
paths explicitly, records the linker file version/hash, and requires the
actual rustc-emitted final executable link invocation to contain these paths.
This is final-link evidence, not tracing of every C compiler/header or loader.
The latest scope is local development verification; Windows 10 is deferred
delivery validation NOT_RUN, while historical BLOCKED evidence is preserved.
Actions are pinned to full commit IDs; Rust is exact. No runtime DLLs are copied
without a demonstrated need and license review. The MSVC CRT is statically
linked through the shared .cargo/config.toml target setting; runtime import
inspection of the final executable remains part of candidate verification.

Packaging includes each dependency's root LICENSE/NOTICE/COPYING materials
where present, NSH repository license, metadata and a license inventory. This
inventory is not a declaration that a complete future product license review
has happened. No component tests exist at TX-00: native lifecycle verification
is separate; G0 is BLOCKED until Win10 22H2 clean build/run evidence exists.
