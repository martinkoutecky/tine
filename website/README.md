# Tine website

The static landing page for Tine, served by **GitHub Pages** from this folder.

- `index.html` / `style.css` — the whole site. No build step, no external fonts,
  no trackers; everything is self-contained and loads from relative paths (so it
  works both at `https://<user>.github.io/tine/` and at a custom apex domain).
- `img/` — screenshots, copied from `docs/img/` (the same curated set the README uses).
- `guide/` — the public **Guide**: the onboarding demo graph published with
  Tine's *own* HTML export as a read-only reference (so the Guide dogfoods the
  publish feature). Generated, not hand-edited — see "Regenerating the Guide"
  below. Linked from the nav/hero as a relative `guide/` path so it works on
  either Pages domain.
- `demo/` — only a small hand-written redirect page remains here, forwarding
  old `/demo/` links to `../guide/`. Not generated content.
- `favicon.svg` — the app-icon mark.
- `.nojekyll` — tells Pages to serve the files as-is (no Jekyll processing).
- `CNAME` — present only once a custom domain is chosen (see below).

## Deploying

`.github/workflows/pages.yml` deploys this folder on every push that touches
`website/**` (or via the Actions tab → Run workflow).

**One-time setup:** repo Settings → Pages → "Build and deployment" → Source:
**GitHub Actions**. The workflow token can deploy to Pages but can't *enable* it
(that needs repo-admin scope it lacks — the symptom is a "Resource not accessible
by integration" failure), so this toggle must be flipped by hand once.

## Custom domain

1. Register the domain (e.g. via forpsi).
2. Add a `website/CNAME` file containing just the bare domain, e.g. `usetine.com`.
   **It must live in this folder** — with Actions-based Pages the CNAME is part of
   the published artifact; setting the domain only in the repo's Pages UI gets wiped
   on the next deploy.
3. Point DNS at GitHub Pages:
   - **Apex** (`example.com`): four `A` records →
     `185.199.108.153`, `185.199.109.153`, `185.199.110.153`, `185.199.111.153`
     (and, recommended, four `AAAA` → `2606:50c0:8000::153` … `:8003::153`).
     forpsi's DNS panel has no ALIAS/ANAME at the apex, so use these A records.
   - **`www`/sub-domain**: one `CNAME` → `<user>.github.io`.
4. In repo Settings → Pages, tick **Enforce HTTPS** once the certificate provisions.

## Regenerating the Guide

`website/guide/` is built from the onboarding demo templates
(`crates/tine-core/src/templates/`) via Tine's HTML export. Re-run after changing
those templates:

```sh
source scripts/env.sh
cargo run -q -p tine-core --example build-guide-site -- website/guide
```

The example scaffolds the demo graph in a temp dir, publishes **all** its pages
(forcing all-pages-public in memory only — the shipped onboarding config stays
private-by-default), rewrites asset paths to be self-contained, and writes the site
into `website/guide/` (overwriting it). `npm run docs:build` / `docs:check` wrap
the same generator with the freshness + link checks.

## Regenerating screenshots

The site reuses `docs/img/*.png`. If those change, re-copy them:

```sh
cp docs/img/{hero,tabs,focus-dim,quick-capture,query,pdf,carry,settings}.png website/img/
# (focus-dim.png is copied to website/img/focus.png)
```
