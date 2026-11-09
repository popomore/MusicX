# Local NSH patch

Based on the published nsh-platform-macos 0.4.0 crate (Apache-2.0).

- The upstream manifest always enables nsh-base/highlight and nsh-base/math.
  This copy gates them behind rich-text, enabled by default to preserve upstream
  behavior. Mini Cloud Music disables default features.
- AppOptions gains decorated (default true). False uses a keyboard-focusable
  borderless window with a transparent background, so a small Rust app can draw
  its own rounded content, drag region, close and minimize controls without
  initializing the system title bar. Normal decorated windows retain upstream
  behavior. Runtime changes are confined to src/app.rs and src/chrome.rs.
- Window close and minimize commands are dispatched after the current mount
  batch. Closing inside the batch otherwise tears down a borrowed surface;
  minimizing there also remains inside its CoreAnimation transaction. The
  borderless subclass implements performClose with the normal delegate check
  because there is no native close button.

The application explicitly registers Button, Slider, Sheet and Popover via set_ui_sets;
its theme and token handling remain provided by nsh-base::registry_of.

- `AppOptions::transparent` (default false) opts a shaped window's host surface
  out of the opaque light/dark fallback in `system::follow_background`.
  The music panel opts in: only its two rounded cards paint backgrounds, while
  their corner cutouts and the outside drawer gap reveal the native window.
  `NSWindow::setOpaque(false)` alone does not clear the framework's container
  layer. This patch touches app.rs, surface.rs and system.rs and leaves ordinary
  embedded/decorated surfaces' default background behavior unchanged.
