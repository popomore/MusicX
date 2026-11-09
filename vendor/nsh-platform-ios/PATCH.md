# NSH iOS 0.4.0 local packaging fix

The published crate references `../../nsh-platform-macos/src/speech.rs`, which
is absent from the crates.io package layout. Include the unmodified speech.rs
from NSH 0.4.0's macOS backend in this crate and use `mod speech` locally.
No runtime behavior or permissions change. Upstream licenses are preserved.
