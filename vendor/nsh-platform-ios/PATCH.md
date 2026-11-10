# NSH iOS 0.4.0 local patches

The published crate references `../../nsh-platform-macos/src/speech.rs`, which
is absent from the crates.io package layout. Include the unmodified speech.rs
from NSH 0.4.0's macOS backend in this crate and use `mod speech` locally.
That packaging fix does not change runtime behavior or permissions. Upstream licenses are preserved.

An opt-in `set_image_transport` bridge routes native UIKit remote image downloads
through the core loader and the application's transport, including disk misses.
This allows MusicX's helper to fetch authenticated artwork. The returned core
tickets preserve cancellation and progress; callers that do not install an
override retain NSURLSession behavior. Authenticated image responses use no-store.
