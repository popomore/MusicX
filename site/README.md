# MusicX website

Static bilingual Astro site for https://musicx.chuo.me. English is at `/`;
Simplified Chinese is at `/zh/`. This directory owns its source, templates,
screenshots, tests, npm lockfile and tooling.

## Development

Use Node 26.13.0 (the repository's CI version) and npm. From this directory:

```sh
npm ci
npm run dev
npm run typecheck
npm run lint
npm test
npm run check:deploy
```

`npm test` builds the site and checks the actual static HTML, assets, download
links, language metadata, sitemap, 404 page and Worker environment separation.
`check:deploy` packages both environments without uploading. It does not verify
Cloudflare credentials, domain ownership or the running website.

See [product facts](docs/product.md) and [screenshot provenance](docs/assets.md).
Download links in `src/product.ts` are pinned snapshots. Verify a replacement
release and installer before changing the URL and version together.

## CI and release

`.github/workflows/site.yml` runs the same checks for site PRs and pushes.
PR checks have read-only permissions and no deployment secrets.

- A site change pushed to `main` deploys the checked build to the existing
  `chuome-musicx-site-test` Worker.
- A human-created stable `site-vX.Y.Z` tag deploys that exact commit to
  `chuome-musicx-site` and its existing `musicx.chuo.me` Custom Domain.
  The tagged commit must be reachable from `main`. Site tags are separate from
  native application versions; this workflow does not generate tags or releases.
- Manual dispatch verifies an existing stable site tag and packages both
  environments. It never deploys.
- Each deployment uses the artifact built and checked in the same workflow run;
  deployments are serialized by environment.

The `test` and `production` GitHub Environments need `CLOUDFLARE_API_TOKEN` and
`CLOUDFLARE_ACCOUNT_ID`. Configure a token for the account and Workers/domains
already named in `wrangler.jsonc`; do not copy credentials into the repository.
See [Cloudflare's CI authentication guide](https://developers.cloudflare.com/workers/ci-cd/external-cicd/github-actions/).
Credential availability and live deployment are separate acceptance steps.
