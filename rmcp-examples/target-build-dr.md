In Rust, the package manager Cargo downloads and compiles dependencies into a **per-project `target/` directory** by default. To centralize them, you have a few options depending on your goal:

---

### 1. Shared registry cache (already global)

The **downloaded source code** of crates is already shared globally at:

```
~/.cargo/registry/
~/.cargo/git/
```

This part is automatic — Cargo never re-downloads the same crate version twice across projects. What isn't shared is the **compiled output**.

---

### 2. Shared compiled artifacts — `CARGO_TARGET_DIR`

Set this environment variable to redirect all build output to one location:

```bash
export CARGO_TARGET_DIR=~/.cargo/target
```

Add it to your shell profile (`~/.bashrc`, `~/.zshrc`, etc.) to make it permanent. Every project then compiles into that single directory instead of its own `target/`.

**Downside:** Cargo uses the target directory as a lock — only one build can run at a time across all projects, and switching between projects with different feature flags or toolchains causes full recompiles.

---

### 3. Per-user Cargo config (recommended)

Rather than an env var, set it in Cargo's global config file:

```toml
# ~/.cargo/config.toml

[build]
target-dir = "/home/yourname/.cargo/target"
```

This is the cleanest approach — no shell environment dependency, applies to every project automatically, and can be overridden per-project by a local `.cargo/config.toml` if needed.

---

### 4. `sccache` — shared compilation cache (best for teams/CI)

[`sccache`](https://github.com/mozilla/sccache) is a compiler cache that works across projects and even machines (with S3/GCS backends):

```bash
cargo install sccache

# ~/.cargo/config.toml
[build]
rustc-wrapper = "sccache"
```

`sccache` caches compiled artifacts by content hash, so if two projects use the same crate at the same version with the same flags, it reuses the compiled object — even if `target-dir` is different per project. This is the most cache-efficient approach and doesn't have the single-lock problem of a shared `target-dir`.

---

### 5. Cargo workspaces (for related projects)

If your projects are related (like the Vehicle Property Store server + gateway + client), use a **workspace**:

```toml
# Cargo.toml at the repo root
[workspace]
members = [
  "property-server",
  "zonal-gateway",
  "property-client",
]
```

All workspace members share a single `target/` directory at the workspace root automatically. Dependencies compiled for one member are reused by others. This is the idiomatic Rust solution for multi-crate projects.

---

### Quick comparison

| Approach | Scope | Reuse across projects | Concurrent builds |
|---|---|---|---|
| Default | Per-project `target/` | No | Yes |
| `CARGO_TARGET_DIR` | Global single dir | Yes (same flags) | No (lock conflict) |
| `~/.cargo/config.toml` | Global, overridable | Yes (same flags) | No (lock conflict) |
| `sccache` | Global cache by hash | Yes | Yes |
| Workspace | Per-workspace | Yes (members only) | Yes (within workspace) |

For a single developer machine, the **`~/.cargo/config.toml` approach** is the simplest win. For a CI environment or team, **`sccache`** gives you the most leverage.
