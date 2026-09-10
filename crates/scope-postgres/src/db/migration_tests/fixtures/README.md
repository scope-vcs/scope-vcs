# Original-chain upgrade fixture

`original_chain_schema.sql` is the schema-only PostgreSQL snapshot captured
from original-chain revision `578bec00da088598919082b35a7153f62bf0b860`, through
`m0042_request_media`, and introduced as the baseline by `e183ddcf`.
It was frozen here on September 10, 2026 from the unchanged snapshot in
`7d3c254c1b4d5f8bfdc2477e7396c882456cb081`.

SHA-256: `484e645f83baf35085e574905f7c2812d35ee3510a4709557f1ef60df4551091`.

Keep this historical fixture unchanged when adding migrations or changing the
candidate schema. Tests install it directly, then seed business rows and the
original ledger. The complete dump is intentionally kept together despite its
size: splitting or regenerating it would obscure its provenance. It contains
schema definitions only, with no production rows or credentials.

This snapshot includes the `retained` upload state. The preflight regression
also recreates the two older production CHECK definitions that omitted it.
That variant reproduces the real September 10 deployment failure and must fail
schema preflight even though its migration names match.
