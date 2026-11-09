# Scope identity

`web/brand/scope.svg` is the canonical Contour artwork. Its named mark and
outlined wordmark groups retain the approved alignment as one composition.
Full-logo placements use the combined SVG; they do not position live text next
to a standalone mark. The mark is centered on the main lowercase letter body,
with the p descender below it.

The default full logo is 24 px high and 93.3617 px wide. The footer uses 20 px
height. Standalone topbar marks use a 20 px canvas. Charcoal and ivory remain
the application colors; ordinary text keeps the application fonts.

## Export

From `web`, with its pinned Node/pnpm dependencies and Playwright Chromium:

```sh
pnpm generate:brand
pnpm check:brand
```

The exporter uses the existing Playwright renderer. It generates the combined
logo, standalone mark, favicon SVG and PNG, touch icon, 512 px provider avatar,
1200 × 630 social preview and 188 × 48 email header. The latter displays at
94 × 24. All web exports live under `/brand/contour-v1/` so old cached artwork
does not survive the initial rollout. The exporter also updates the two inline
symbols in the standalone repository homepage, `README.html`.

`pnpm test:smoke` checks generated exports and exercises the actual logo
components at phone and desktop widths, in both themes and at 1×/2× density.
The browser spec checks the rendered mark/letter-body centers, open contour
gaps at small icon sizes, aspect ratio, and font-independent rendering.

The outlined wordmark uses Adwaita Sans. Its source and SIL Open Font License
notice are recorded in `legal/copied-sources.json` and the generated public
third-party notices. No font installation is needed to render the logo.

## Provider-managed artwork

Release the web assets before the API update so new invite emails can load their
header. After the web release publishes these assets, use the 512 px avatar for square
identity surfaces and the combined logo for horizontal slots. Inspect Clerk
hosted account pages, Clerk email templates, the production GitHub App listing
and authorization screen, and the scope-vcs organization avatar for old art.
Use a PNG where the provider does not accept SVG. Verify the provider's saved
result, its dark/light treatment, and a newly opened authorization or email
preview. Never point a production setting at an asset URL before that URL is
available in the released application.
