+++
title = "How to Contribute"
description = "Contribute to rer — report bugs, send patches, improve documentation."
date = 2026-05-15T18:10:00+00:00
updated = 2026-05-15T18:10:00+00:00
draft = false
weight = 410
sort_by = "weight"
template = "docs/page.html"

[extra]
lead = "Contribute to rer — report bugs, send patches, improve documentation."
toc = true
top = false
+++

👉 Make sure to read the [Code of Conduct](../code-of-conduct/).

## Contribute code

👉 The rer source lives at [`doubleailes/rer`](https://github.com/doubleailes/rer).

- Follow the [GitHub flow](https://guides.github.com/introduction/flow/).
- Follow the [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) specification.

### Commit scopes

The commit scope is the crate name without the `rer-` prefix:

- `feat(version): …`
- `fix(resolver): …`
- `chore(python): …`
- `docs: …`
- `ci: …`

### Before you push

```bash
cargo fmt --all
cargo build
cargo test
```

For changes that touch the solver, also run the rez differential
benchmark (it needs the `rez` git submodule and several minutes):

```bash
git submodule update --init
python scripts/prepare_benchmark_data.py
cargo test --release -p rer-resolver --test test_rez_benchmark -- --ignored
```

The `benchmark` CI workflow runs this on every PR touching the resolver.

### Create an issue

- [Bug report](https://github.com/doubleailes/rer/issues/new)
- [Feature request](https://github.com/doubleailes/rer/issues/new)

## Improve documentation

This site lives in [`docs/`](https://github.com/doubleailes/rer/tree/main/docs)
and is built with [Zola](https://www.getzola.org/). The page-level
content is in `docs/content/`; the configuration is in `docs/config.toml`.

```bash
cd docs
zola serve
```

The README and `CLAUDE.md` at the repo root are the authoritative
descriptions of the code; if you spot drift between those and this site,
the README/CLAUDE.md wins — please send a PR.

## Reporting a regression against rez

If you find a request that resolves differently in `rer` than in `rez`,
that is a bug. Please include:

- the request list (e.g. `["python-3", "maya-2024"]`)
- a minimal package repository that reproduces the divergence
- rez's resolution and rer's resolution

The 188-case rez benchmark is the authoritative reference — if a case
from that suite ever diverges, that's a release blocker.
