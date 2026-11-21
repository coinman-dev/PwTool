# eframe 0.36.2 with a patch for Password Tool

Unmodified copy of the `eframe` 0.36.2 crate from crates.io (license MIT OR Apache-2.0,
`LICENSE-MIT`, `LICENSE-APACHE` from the egui repository at tag `0.36.2`), used through
`[patch.crates-io]` in the workspace `Cargo.toml`, with one change:

`src/native/glow_integration.rs` — AccessKit (screen readers) for every viewport. Upstream
creates the AccessKit adapter for the root viewport only, so screen readers see the other
windows (PassCube, Configuration, dialogs) as empty. The patch keeps the event loop proxy
once the root has its adapter (`GlutinWindowContext::accesskit_proxy`) and gives each new
window of another viewport its own adapter: the window is created hidden, the adapter is
added, then the window is shown (accesskit_winit requires the adapter before the window is
first shown). All changes are marked "PwTool patch".

When updating eframe: check whether upstream supports AccessKit for all viewports
(then remove this copy and the `[patch.crates-io]` entry), else copy the new version and
apply the change again (`diff` against the crates.io source shows it).
