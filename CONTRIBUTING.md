# Contributing to FerrisCore

FerrisCore is the minimal base of Ferris (9 crates). Thank you for contributing!

## Getting started

1. Fork and clone this repository **and** its sibling
   [`FerrisFramework`](https://github.com/OpenFerris/FerrisFramework):

   ```bash
   mkdir ferris && cd ferris
   git clone https://github.com/<you>/FerrisCore.git
   git clone https://github.com/<you>/FerrisFramework.git
   ```

2. Read [`AGENTS.md`](AGENTS.md). It is the operating contract: container,
   gates, layout, branches and release.
3. Branch from `develop` (`feature/…`, `fix/…`) and open the PR against
   `develop`. `master` is production.

## Gates

Everything runs inside the development container:

```bash
docker compose run --rm ferris cargo fmt --all -- --check
docker compose run --rm ferris cargo clippy --workspace --all-targets
docker compose run --rm ferris cargo check --workspace --all-targets
docker compose run --rm ferris cargo test --workspace -j 2 -- --test-threads=1
```

A PR must add zero new warnings and include a regression test for every bug fix.

## API changes

FerrisCore is consumed by FerrisFramework through relative paths. A change to a
public type (errors, `Module`, WIT-consumed structs, metadata) ships here first;
the Framework PR follows with the same branch name and adjusts the consumer.

## Code of conduct

See [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md).
