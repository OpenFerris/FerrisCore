# AGENTS.md — FerrisCore

Operating contract for any agent (or human) working on **FerrisCore**, the base
Rust workspace of the Ferris project. It describes the tree as it exists today;
verify before trusting it and update it when reality changes.

Read §0 (container rule), §1 (compaction) and §3 (sibling repository) first.

---

## 0. HARD RULE — build inside the project container

FerrisCore builds inside the **pinned image** defined by this repository, not
with the host toolchain.

| Fact | Value |
|---|---|
| Image | `ferris-dev`, built from `Dockerfile` |
| Orchestration | `compose.yaml` (service `ferris`, PostgreSQL, Redis) |
| Workspace | bind-mounted at `/workspace` |
| Toolchain | `rust-toolchain.toml` pins **1.98.1** |
| Targets | `wasm32-unknown-unknown`, `wasm32-wasip2` |
| Also inside | Node 24, `wasm-bindgen-cli` 0.2.128, `cargo-leptos`, `wasm-tools` |
| User | non-root `ferris`; `USER_ID`/`GROUP_ID` are build args (default 1000) |

### Canonical way to run a command

```bash
docker compose run --rm ferris cargo check --workspace --all-targets
```

Define it once per session:

```bash
dx() { docker compose run --rm ferris sh -lc "$*"; }
dx cargo test -p ferris-document --lib
```

If you are already inside the devcontainer (`.devcontainer/devcontainer.json`),
run the command directly: the workspace and toolchain are the same.

Fallback without Compose (same image):

```bash
docker build -t ferris-dev .
docker run --rm -it -v "$PWD:/workspace" -w /workspace ferris-dev cargo check
```

### Session sanity check (run first)

```bash
docker compose run --rm ferris sh -lc 'rustc -V && cargo -V && rustup target list --installed'
```

Must print `1.98.1` for both `rustc` and `cargo` and both WASM targets.

### Forbidden

- **Never** run `cargo`, `rustc`, `rustup`, `node`, `npm` or `tailwindcss` on the
  host.
- **Never** run them in another checkout or container with a different toolchain.
- **Never** hand-edit `target/` or copy build artifacts between host and
  container.
- **Never `cargo clean`** (it deletes the shared dependency cache). Remove only
  the profile you need before a large rebuild, and check `df -h .` first.

Allowed on the host: `git`, `docker`/`docker compose` (to run the project image),
read-only inspection (`ls`, `cat`, `rg`, `grep`, `find`, `git diff`) and editors.
Run a language server inside the devcontainer, not on the host.

---

## 1. Compaction & continuity protocol

Long tasks get compacted. A checkpoint, handoff or summary MUST carry, in this
order of priority:

1. **The main task, verbatim and complete**, including every clause of a
   multi-part order. Do not paraphrase scope away or narrow it to the last
   subtask.
2. **What is needed to avoid re-exploring**: files/symbols already read
   (`path:line`), commands run and their results, artifacts created or changed,
   decisions taken and why.
3. **Every explicit rule the user gave** (language, paths, prohibitions, "one
   file", "don't touch X") plus the rules in this file that bear on the task.
   User rules outrank your defaults.
4. **Current position and next step**: done, in progress, next concrete action,
   blocked items, unverified claims.

Point at canonical docs and commits instead of pasting large content. Never
claim done what you did not verify (§7). Keep it small but lossless.

---

## 2. What this repository is

FerrisCore is the **minimal base** of Ferris: metadata, persistence, document
engine, HTTP façade, authentication and the shared frontend layer. It has **no
vertical, no host wasm, no server, no CLI and no Desk application**.

| Crate | Responsibility |
|---|---|
| `ferris-core` | Error taxonomy, config, hooks, permissions primitives, tenant, workspaces, themes, values, ports (`QueuePort`, `MailerPort`, `ServiceHandles`). |
| `ferris-macro` | Proc-macros `#[doctype]`, `#[workflow]`, `#[permission]`, `#[job]`. |
| `ferris-db` | Pool, dialect (sqlite/postgres/mysql), SQL builder, migrations, the single `sqlx::Error` classifier. |
| `ferris-schema` | Registry, metadata (`contract::meta` is the portable wire subset), customizations, migration runner. |
| `ferris-document` | Document engine: typed `crud.rs`, dynamic `engine.rs`, lifecycle, workflow, versioning, naming, validation. |
| `ferris-api` | REST façade, `authz/`, the native `Module` contract, notification port (`notify::Notifier`). |
| `ferris-auth` | Sessions, OAuth2/PKCE, MFA, password hashing. |
| `ferris-web-shared` | Wire contract + metadata helpers for the browser. **No Leptos, no `web-sys`.** |
| `ferris-web-ui` | ~90 UI components (`components/ui`), hooks (`components/hooks`), style helpers. **No routing, no fetching, no engine bridge.** |

Workspace: `resolver = "2"`, edition 2024, version **0.3.0**, Rust **1.98.1**,
MIT. All 9 members are `default-members`, so a plain `cargo check` compiles the
whole base.

### Dependency direction is load-bearing

- `ferris-core` is the root; nothing here depends on a vertical.
- `ferris-api` **must not** depend on `ferris-print`, `ferris-report`,
  `ferris-realtime`, `ferris-files`, `ferris-email`, `ferris-integrations` or
  `ferris-wasm`. `ferris-api/tests/architecture.rs` guards the manifest.
- Frontend: `ferris-web-ui → ferris-web-shared`. Never invert it.
- `unsafe` is forbidden by default. If it is truly required: a `// SAFETY:` block
  per invariant, a test that exercises it and an explicit note in the report.
- No panics on library paths: no `unwrap()`, `expect()`, out-of-bounds indexing
  or `panic!` reachable from a request. Return a typed error.
- No silent success: a validator that returns `Ok(())`, a hook that does
  nothing or a flag parsed and ignored is a defect.

### Errors

One taxonomy in `ferris-core/src/error.rs`. Domain errors are `thiserror`.
`anyhow` stays at the binary/orchestration layer (it does not belong in this
repository). Classify each external failure exactly once at the boundary;
`ferris-db/src/error.rs::classify` is the model.

### File size and modular layout

- Any hand-written source file above **600 lines** must be split in the same
  change (folder + `mod.rs` that declares and re-exports; the `mod.rs` holds no
  implementation). Generated code and a single long `match`/data table may be
  stated exceptions.
- A `#[cfg(test)]` suite moves to `foo/tests.rs`; a file holds code or tests,
  not both.
- Behaviour-preserving: same public API, same module paths (`pub use`), same
  visibility, no new `#[allow]` to force it through.

### Dependencies and lint bar

- New dependencies go in `[workspace.dependencies]` with a written
  justification (what it replaces, licence, size/build impact). Prefer
  **rustls**; avoid `native-tls`/OpenSSL.
- `Cargo.toml` sets `unused_crate_dependencies`, `unused_imports`,
  `unused_variables`, `dead_code` and the full clippy set to **`warn`**.
  Warnings are failures: a PR adds zero new ones.
- Never silence a lint with a bare `#[allow(...)]`; add the allow with a comment
  that states why, next to the item.
- `ferris-db` uses in-memory SQLite for unit tests (`test_support.rs`). Never
  point a test at a developer's real database. No network except `wiremock`.

### Tests

- Unit tests inline in `#[cfg(test)] mod tests` (or a `tests.rs` fragment) next
  to the code; integration tests in `<crate>/tests/`.
- Deterministic: inject clocks, seed fixed UUIDs, no sleeps. Assert on behaviour
  and error **variants**, not log strings.
- Every bug fix gets a regression test that fails without the fix.

---

## 3. Sibling repository: FerrisFramework

FerrisCore is one half of the product. **FerrisFramework** (Desk, verticals,
host wasm, server, CLI) depends on these crates by relative path:

```text
../FerrisFramework/ferris-cli/Cargo.toml
    ferris-core = { path = "../../FerrisCore/ferris-core" }
```

Therefore:

- The two checkouts must be **siblings** on disk (`.../ferris/FerrisCore` and
  `.../ferris/FerrisFramework`).
- **Core never depends on Framework.** If a change seems to need it, the seam is
  wrong: move the primitive into Core or consume a port.
- The WIT contract (`ferris-wasm/wit/ferris.wit`), the wasmtime host and the
  `Module` composition root live in Framework. Core only defines the traits and
  types those layers consume.
- A change that crosses the boundary is prepared on same-named branches in both
  repositories, with the Core PR first (Framework CI checks out `FerrisCore`).
- Framework CI checks out this repository as the sibling `FerrisCore` next to its
  `framework/` checkout, so `../../FerrisCore` resolves; each job checks out both
  repositories. A `master` or `develop` break here blocks it. Do not push a red
  base branch.

---

## 4. Definition of Done (gates)

Run everything inside the dev container.

| Gate | Command | When |
|---|---|---|
| Format | `cargo fmt --all -- --check` | always |
| Lint | `cargo clippy --workspace --all-targets` | always |
| Typecheck | `cargo check --workspace --all-targets` | always |
| Tests | `cargo test --workspace -j 2 -- --test-threads=1` | always |
| Crate lint | `cargo clippy -p <crate> --all-targets -- -D warnings` | when touching one crate |
| Diff review | `git diff` | always — reread your own diff |

Notes:

- The test suite runs with `-j 2 -- --test-threads=1`: full parallelism exceeds
  the available memory and gets OOM-killed. This is a memory limit, not a flaky
  workaround.
- The HH/SSR/CLI gates live in Framework's `AGENTS.md`; they are not applicable
  here.
- If a gate cannot be run (no DB, no network, missing tool), say so explicitly
  and report it as **unverified**. Never claim green what you did not run.

---

## 5. Repository hygiene and branches

- Branch layout (same in all three repositories):

| Branch | Role |
|---|---|
| `develop` | **Default.** Integration; every working PR targets it. |
| `master` | Production. Only `release/*` and `hotfix/*` merges land here. |
| `feature/*`, `fix/*` | Daily work, branched from `develop`. |
| `release/*` | Freeze before publishing; merged into `master` and back into `develop`. |
| `hotfix/*` | Urgent patch from `master`; merged into `master` and `develop`. |

- Protected branches: direct pushes to `master` and `develop` are disabled; a PR
  and the `CI` workflow are required. The required checks are `Format`, `Check`
  and `Test`.
- Commit subjects: `feat(scope): …`, `fix(scope): …`, `refactor(…): …`,
  `docs(…): …`, `wip: …`. The body states what changed and why.
- `Cargo.lock` is **tracked** and updated by `cargo`, never by hand.
- Never commit `target/`, `test.db`, `*.db`, `node_modules/` or generated
  artifacts. `vendor/serde` and `vendor/serde_core` are `[patch.crates-io]`
  overrides: do not remove or "clean up" the patch.
- A local `test/` scaffold is git-ignored and excluded from the workspace. Do not
  wire it in.
- Do not run a formatter over the whole repository as a side effect of an
  unrelated change.

---

## 6. Release

The version is a single number shared by both workspaces and the WIT contract
(`ferris:framework@X.Y.Z`). While the project is `0.x` it is pre-1.0; a patch
does not touch WIT, a minor adds compatible WIT surface, a major breaks it. The
full policy is `VERSIONING.md` in FerrisFramework.

1. Branch `release/<version>` from `develop`.
2. Bump `[workspace.package] version` in **both** `Cargo.toml` files, both copies
   of `ferris.wit`, `FerrisDocs/project.json` and `CHANGELOG.md`.
3. Run every gate in §4, plus Framework's gates.
4. Open a PR from `release/<version>` into `master`; merge with a merge commit.
5. Tag `v<version>` on `master` and push the tag.
6. Merge `master` back into `develop` so the bump is not lost.
7. Framework's `wit_version` test must pass before the tag is considered valid.

---

## 7. Verification discipline

1. **Locate by grep, confirm by reading.** No claim rests on a text match alone;
   every `file:line` was actually read.
2. **Negatives are proven by absence.** "Nobody calls it" requires
   `grep -rn "<symbol>" --include='*.rs' .` across all crates, with the result
   quoted.
3. **Report what was not verified.** Limitations belong in the answer.
4. **Do not invent APIs.** Read the trait, the impl and the call site before
   using them.
5. When you change something, note what was already correct and must not be
   disturbed.

Final report: what changed, which files, the exact commands run inside the
container and their result, and anything left unverified or knowingly skipped.

---

## 8. Documentation map

| File | What it is |
|---|---|
| `AGENTS.md` | This contract. |
| `README.md` | Public entry point for the repository. |
| `../FerrisFramework/AGENTS.md` | Contract for Desk, verticals, wasm host, server and CLI. |
| `../FerrisFramework/VERSIONING.md` | One-version policy and release checklist. |
| `../FerrisDocs/` | Public documentation site (Astro + Starlight, Spanish prose). |

---

## 9. One-page checklist

- [ ] Everything ran inside the dev container (or was read-only on the host).
- [ ] Sanity check printed `1.98.1`.
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --workspace --all-targets` adds no new warnings.
- [ ] `cargo check --workspace --all-targets` passes.
- [ ] `cargo test --workspace -j 2 -- --test-threads=1` passes, with a
      regression test for any bug fixed.
- [ ] No new `unwrap`/`expect`/panic on request paths; errors go through the
      `ferris-core` taxonomy.
- [ ] No `unsafe`, no new dependency without justification.
- [ ] No file grew past 600 lines (or the change states why it could not be
      split).
- [ ] If the API manifest or a vertical seam changed, the Framework PR exists
      and its CI checks out this branch.
- [ ] `git diff` reread; no `cargo clean`, no host artifacts, no `target/` noise.
- [ ] If context was compacted: the summary keeps the task, the no-re-exploration
      context, the explicit rules and the next step.
