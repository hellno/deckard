# Golden references

These HTML files are the **pixel ground-truth** the Deckard UI is built against. `DESIGN.md` holds
the rules; these hold the pixels. When the two disagree, the doc wins on rules and the reference wins
on pixels — fix whichever is stale.

| file | covers |
|---|---|
| `deckard-editorial-v3.html` | wallet home, send confirm, swap compose, swap review, activity — the editorial language, light + dark |
| `deckard-agent-v4.html` | the agent surface, the compact agent presence on the home, and the transaction-as-hero confirm (this confirm supersedes v3's send confirm) |

Open either in a browser to see the intended layout, hierarchy, and both themes. The GPUI app should
match these in layout and hierarchy — that's the last item in the `DESIGN.md` visual definition of done.

## Why they're here now

Through v2 these lived only under `~/.gstack/projects/hellno-deckard/designs/`, per-user and
unversioned, so "matches the golden reference" was a check nobody could actually run and the pointer
to them went stale. v3 checked the two current references into the repo so the definition-of-done item
is verifiable and so a reviewer on any machine sees the same pixels.

Superseded explorations (`deckard-cockpit-v2.html`, `deckard-foundation-preview.html`,
`elevate-home-variants.html`) and the competitor reference screenshots were **not** copied in — they
were the scaffolding that produced these two, not the ground-truth. They remain under `~/.gstack/...`
for history.

## Updating a reference

These are static artifacts, not build output. When the design direction changes, regenerate the
affected file (the design tooling under `~/.gstack/...` produced them), replace it here in the same PR
that changes `DESIGN.md`, and note the change in the `DESIGN.md` decisions log so the doc and the
pixels move together.
