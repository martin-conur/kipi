---
name: rust-style
description: "Use when writing or reviewing Rust in personal projects: error handling, module layout and testing style."
x-kipi:
  scope: personal
  tags: [rust]
---

# rust-style

- Libraries return `thiserror` enums; binaries use `anyhow` at the edge only.
- No `unwrap()` outside tests; `expect("why this cannot fail")` when it truly cannot.
- One module per concept; keep `lib.rs` a table of contents.
- Unit tests live next to the code; integration tests exercise the public API only.
