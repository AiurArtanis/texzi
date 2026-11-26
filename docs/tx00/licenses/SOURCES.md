# Supplemental license sources

Retrieved from the exact repository commit reported by .cargo_vcs_info.json
in the locked crates.io package (root LICENSE):

- rust-i18n macro/support 4.2.4: longbridge/rust-i18n,
  5f3ea5ba03395f2f2908e3587e01788cfdfd3555.
- taffy 0.15.1: DioxusLabs/taffy,
  0064bd54cbe38bf1d8bcd6ec6ab0704d45fb048a.
  Taffy 0.16.0 ships the same MIT license text in its source repository
  at commit 497de49c200daee81a18ce627bec98d5340ae57a
  (LICENSE blob f6b3d8f95569d0dfca076b6b24901b2a4a89aca8).
- webview2-com/sys 0.39.1: wravery/webview2-rs,
  edc2caf886175ccaebe86078c9cfe1ae2a187328.
- webview2-com-macros 0.8.1: wravery/webview2-rs,
  dffa41a8a46d3f5565eefbff2de57d38d399f158.
- wuff 0.2.10: nicoburns/wuff,
  0bb77cb3448100b8f560abf796014c8b85d75e23.

mac 0.1.1 ships no license file or VCS identity. Its shipped README identifies
Jonathan Reem as author and explicitly licenses MIT/Apache-2.0. Package that
README together with Apache-2.0 text; select Apache-2.0. No current branch is
substituted for a historical package source.

All Cargo packages are inventoried in sbom.json; framework and crate license
files are copied from fetched locked sources, and Rust's shipped runtime
copyright/license directory is included. This is a build dependency inventory,
including build-time packages, not a claim each entry is linked into the exe.
