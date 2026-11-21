# Texzi patch to markdown 1.0.0

Source is the published `markdown-1.0.0.crate` from crates.io. SHA-256
`a5cab8f2cadc416a82d2e783a1946388b31654d391d1c7d92cc1f03e295b1deb`.
The upstream MIT license is in `license`.

`src/util/edit_map.rs` indexes pending edits by original event position.
Positions are consumed in the same order, and additions and removals at
the same position retain their original prepend, append, and sum rules.
