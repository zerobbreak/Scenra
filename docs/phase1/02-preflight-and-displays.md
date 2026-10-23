# Recording preflight and display validation

Before a scene starts, Scenra checks its initial page state and reads free physical RAM. Below 1024 MB the prompter stays ready, explains the risk of dropped frames, and offers Check again or Record anyway. An override applies only to that start. No take folder or attempt is consumed while waiting for the memory decision. Automated recorder/matrix calls remain advisory and do not display interactive prompts.

Prompter compact/peek resizing is fitted by Rust using the saved zoom and the current monitor work area. Phase changes preserve a dragged position and the selected size; oversized peek windows shrink to fit.

The OBS matrix display selector can exercise each connected display without changing Windows scaling. It temporarily moves the target and selects the monitor in OBS, runs the four WGC/DXGI exclusion rows, and restores both source settings and target placement afterward. Monitor identity is checked against its device interface ID, not just its resolution. Stop any recording before running the matrix. The local Lumina fixture can be used in Chrome as well as WebView2.

Commands:

```sh
pnpm --filter scenra test
pnpm --filter scenra build
cargo clippy -p scenra --all-targets -- -D warnings
cargo test -p scenra --lib
cargo test -p scenra --lib chrome_preflight_then_first_click -- --ignored --nocapture
```

The Chrome test serves the bundled fixture on a temporary loopback HTTP port, deliberately delays page delivery, waits for paint/layout settlement, then verifies that the first native pointer click opens the recording panel. It launches Chrome with a separate test profile; no AI or external site is used.
