# Scope contribution rules

This repository dogfoods Scope. GitHub remains the source of truth for merges,
CI gating, deploys, and releases.

Maintainers mirror every branch they push to GitHub, every PR they open, and
every merge to main to their `scope` remote, following the dogfood-scope skill
from their personal agent skills. The skill is not part of this repository, and
contributors without it or without a `scope` remote have nothing to mirror. A
Scope failure never blocks GitHub delivery; it is noted in the PR description.

Main on Scope only ever receives commits already on GitHub's main.
