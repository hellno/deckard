# Deckard — agent notes

## Design System
Always read `DESIGN.md` before making any visual or UI decision. Fonts, colors, spacing,
the two-signal actor model (amber = human, cyan = agent), the sidebar/contextual-views IA,
component states, and the clear-signing / seed-reveal trust affordances are defined there.
Do not deviate without explicit user approval. In QA/review, flag anything that doesn't match
`DESIGN.md`.

Ground all design work in **real reference screenshots** (Linear, Conductor, Splits, Superhuman,
Stripe), never in remembered descriptions — that is how the first drafts went wrong. The
interactive, dogfooded reference lives at
`~/.gstack/projects/hellno-deckard/designs/deckard-foundation-preview.html`.
