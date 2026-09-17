# Scope contribution rules

This repository dogfoods Scope. GitHub remains the source of truth for merges,
CI gating, deploys, and releases.

Every branch pushed to GitHub, every PR opened, and every merge to main is
mirrored to the `scope` remote following the dogfood-scope skill. A Scope
failure never blocks GitHub delivery; it is noted in the PR description.

Main on Scope only ever receives commits already on GitHub's main.
