# ClinPatch TODO

Plan for porting the prototype in `/Users/madhavajay/dev/clinvar` into this repo, then turning it into a GitHub-runnable ClinVar patch and static range-query publishing system.

## Current Baseline

- Target repo: `git@github.com:madhavajay/ClinPatch.git`, branch `main`, no initial commit yet.
- Source prototype: `/Users/madhavajay/dev/clinvar`.
- Prototype already has:
  - Rust CLI package `clinvar-tool`.
  - `import` into SQLite.
  - `diff` from old VCF to new VCF with `manifest.json` and `changes.jsonl.gz`.
  - `apply-patch` into SQLite.
  - `write-plain` for plain VCF output.
  - `index-rows`, `index-ids`, and `index-positions` for static HTTP byte-range lookup.
  - `rows-local` for local byte-range testing.
  - `serve` static HTTP server with `Range` support.
  - Browser demo in `public/`.
- NCBI GRCh38 source to start with:
  - Latest symlink/current file: `https://ftp.ncbi.nlm.nih.gov/pub/clinvar/vcf_GRCh38/clinvar.vcf.gz`
  - Current index: `https://ftp.ncbi.nlm.nih.gov/pub/clinvar/vcf_GRCh38/clinvar.vcf.gz.tbi`
  - Current checksum: `https://ftp.ncbi.nlm.nih.gov/pub/clinvar/vcf_GRCh38/clinvar.vcf.gz.md5`
  - Dated release files also exist, for example `clinvar_YYYYMMDD.vcf.gz`.

## Product Goal

Build a Rust CLI and GitHub automation that can:

1. Pull the latest ClinVar GRCh38 VCF from NCBI.
2. Detect whether it is newer than the latest release already tracked by this repo.
3. Generate a patch from the previous release to the new release.
4. Generate chunked/static files and indexes that can be queried using ordinary HTTP range requests.
5. Commit the generated patch metadata and publishable static manifests back to this repo.
6. Publish larger generated artifacts in a GitHub-compatible way.
7. Provide CLI tools for slicing, filtering, diffing, patching, and exporting ClinVar data.

## Two Update Products

- [ ] Hosted static shard update.
  - Input: old chunk manifest and new chunk manifest.
  - Compare per-shard SHA-256 values.
  - Reuse unchanged shard files.
  - Upload/commit only changed, added, and removed shard manifests/files.
  - The GitHub Action should update the hosted static view without rewriting every chunk.
- [ ] Standalone release patch.
  - Input: old full release and new full release.
  - Output: `data/patches/GRCh38/OLD_to_NEW/manifest.json` and `changes.jsonl.gz`.
  - Users can fetch this patch independently of the hosted static shard layout.
  - This remains useful for local SQLite snapshots, local VCF-derived snapshots, and offline updates.

## GitHub Data Strategy

- [ ] Do not commit raw full ClinVar downloads directly to normal Git history unless file sizes are confirmed safe.
  - GitHub warns over 50 MiB and blocks files over 100 MiB.
  - GitHub Pages published sites are limited to 1 GB.
  - Full current GRCh38 `clinvar.vcf.gz` is about 187 MiB, so it cannot be committed as a normal Git file.
- [ ] Commit small, useful generated files to the repo:
  - Release manifests.
  - Patch manifests.
  - Checksums.
  - Chunk manifests.
  - Small indexes if they stay below limits.
  - Demo fixtures.
- [ ] Publish large data through one of these paths:
  - GitHub Releases assets for full patch/chunk bundles.
  - A dedicated `gh-pages` artifact only if total published size stays below Pages limits.
  - Optional later: R2/S3/object storage if GitHub Pages/Releases become a poor fit.
- [ ] Keep every generated static chunk below 50 MiB where practical, and always below 100 MiB if it will be committed.

## Published File Sets

The repo should treat generated ClinVar artifacts as multiple related file sets, not one universal bundle.

- [ ] Source metadata set.
  - Release manifests.
  - NCBI source URLs.
  - Source hashes and sizes.
  - Fetch and generation timestamps.
- [ ] Patch set.
  - `changes.jsonl.gz`.
  - Patch manifest.
  - Patch verification metadata.
  - Optional shard replacement manifest.
- [ ] Coordinate static set.
  - Primary HTTP range-query data layout.
  - Plain VCF chunks split by chromosome and coordinate bins.
  - Root coordinate manifest.
  - Position indexes that route to `{ data_file, offset, length }`.
- [ ] ID lookup set.
  - Global Variation ID to chunk/offset map.
  - Shard-local ID byte indexes.
  - Optional compact binary form later if JSON becomes too large.
- [ ] Row lookup set.
  - Global row-to-chunk checkpoint index.
  - Shard-local row-byte indexes.
  - Cumulative row counts in the root manifest.
- [ ] Demo fixture set.
  - Small committed VCF sample.
  - Small committed indexes.
  - GitHub Pages demo uses this first, then can point to larger release assets.
- [ ] Tooling/test fixture set.
  - Tiny two-release VCF fixtures.
  - Expected patch output.
  - Expected chunk/index manifests.

## Phase 1: Port The Prototype

- [ ] Copy source code from `/Users/madhavajay/dev/clinvar` into this repo.
  - `Cargo.toml`
  - `Cargo.lock`
  - `src/main.rs`
  - `README.md`
  - `public/`
  - `server.sh`
- [ ] Rename/package the binary as `clinpatch` or decide to keep `clinvar-tool`.
- [ ] Add `.gitignore`.
  - Ignore `target/`.
  - Ignore downloaded VCF files.
  - Ignore SQLite work databases.
  - Ignore generated full-size release/chunk outputs unless explicitly intended.
- [ ] Add an initial README specific to ClinPatch.
- [ ] Build locally with `cargo build --release`.
- [ ] Run a smoke test against a small VCF slice.
- [ ] Commit the port as the repo baseline.

## Phase 2: Fetch Latest ClinVar

- [ ] Add `clinpatch fetch`.
  - Default assembly: `GRCh38`.
  - Default source: NCBI `vcf_GRCh38`.
  - Download `clinvar.vcf.gz`, `.tbi`, `.md5`, and error report if present.
  - Resolve the dated release id from the dated file name or VCF metadata.
  - Store downloads under ignored local cache path, for example `.clinpatch/cache/GRCh38/YYYY-MM-DD/`.
- [ ] Verify downloads.
  - Check MD5 from NCBI.
  - Check `.tbi` exists and is newer/same release.
  - Record file size, hash, URL, and fetch timestamp.
- [ ] Add `clinpatch latest`.
  - Print latest remote release.
  - Print latest locally known release from `data/releases/manifest.json`.
  - Exit code should make GitHub Actions easy: `0` no update, `10` update available.
- [ ] Add `clinpatch inspect`.
  - Report VCF version, assembly, contigs, header count, record count when available, and key ClinVar INFO fields.

## Phase 3: Release Manifests

- [ ] Define `data/releases/manifest.json`.
  - Assembly.
  - Latest release id/date.
  - List of known releases.
  - Source URLs.
  - Source hashes.
  - Generated artifact paths/URLs.
  - Previous release pointer.
- [ ] Define per-release manifest at `data/releases/GRCh38/YYYY-MM-DD/manifest.json`.
  - Source VCF URL, `.tbi` URL, `.md5` URL.
  - Source file hashes and sizes.
  - Record count.
  - Header hash.
  - Generated chunks.
  - Generated indexes.
  - Patch-from-previous path.
- [ ] Add manifest validation command.
  - `clinpatch validate-manifest data/releases/manifest.json`

## Phase 4: Patch Generation

- [ ] Port and harden prototype `diff`.
  - Compare by ClinVar Variation ID by default.
  - Preserve fallback key behavior for missing IDs.
  - Emit `added`, `removed`, `changed`, and later `moved`.
  - Preserve raw VCF row for exact reconstruction.
- [ ] Add stronger patch manifest fields.
  - Old/new release ids.
  - Assembly.
  - Source hashes.
  - Patch file hashes.
  - Counts by operation.
  - Tool version.
  - Generated timestamp.
- [ ] Add `clinpatch patch create`.
  - Inputs: old cached VCF, new cached VCF.
  - Output: `data/patches/GRCh38/OLD_to_NEW/manifest.json` plus `changes.jsonl.gz`.
- [ ] Add `clinpatch patch apply`.
  - Apply patch to SQLite snapshot or VCF-derived snapshot.
  - Verify output count and hashes.
- [ ] Add `clinpatch patch verify`.
  - Reconstruct or spot-check new release from old release plus patch.
  - Fail CI if counts or hashes do not match.

## Phase 5: Static Chunks For HTTP Range Queries

- [ ] Decide first chunk strategy.
  - First implementation: chromosome plus coordinate-bin chunks, because the existing HTTP range demo is naturally position-oriented.
  - Avoid chromosome-only files if any chromosome shard becomes too large for GitHub/Pages/Releases constraints.
  - Use deterministic bins such as `chr1/000000000-009999999.vcf`, or target-size bins that never split rows.
  - Keep Variation ID as a lookup index inside each chunk, not as the primary chunk layout.
- [ ] Add `clinpatch chunks build`.
  - Input: latest GRCh38 VCF.
  - Output: `data/static/GRCh38/YYYY-MM-DD/chunks/`.
  - Chunk records deterministically by chromosome and coordinate bin.
  - Preserve VCF row order within every chunk.
  - Keep per-chunk file size under configured limit.
- [ ] Generate per-chunk indexes.
  - Row-byte index for row windows.
  - ID index for exact Variation ID lookup.
  - Position index for exact chromosome/position lookup.
  - Optional interval index later.
- [ ] Generate root static manifest.
  - Chunk path/URL.
  - Assembly.
  - Chromosome/contig.
  - Coordinate start/end.
  - Optional Variation ID min/max hints.
  - Row count.
  - Byte size.
  - SHA-256.
  - Index paths.
  - Cumulative row offsets if global row-number lookup is required.
- [ ] Generalize sidecar indexes from single-file to multi-file routing.
  - Current prototype indexes contain one `data_file` and byte offsets into that file.
  - Production indexes can instead map each lookup result to `{ data_file, offset, length }`.
  - A root position index can point directly at chromosome/coordinate chunk files.
  - A root Variation ID index can point directly at the chunk file and byte range for that record.
  - A root row index can map global row windows onto one or more chunk-local byte ranges.
  - Keep shard-local indexes too, so clients can either load one global index or only the index for selected shards.
- [ ] Add root query routing.
  - Region query chooses all overlapping coordinate chunks.
  - Exact position query chooses one coordinate chunk except boundary-spanning records.
  - Variation ID query uses a compact global ID-to-chunk map, then the shard-local ID byte index.
  - Global row query uses cumulative row counts to choose one or more chunks.
- [ ] Add `clinpatch chunks diff`.
  - Compare old and new chunk manifests.
  - Emit changed/added/removed/unchanged chunk list.
  - Prefer replace-whole-chunk patching for static clients.
- [ ] Add `clinpatch chunks verify`.
  - Confirm all chunk hashes match.
  - Confirm row counts sum to release count.
  - Confirm every indexed byte range maps to the expected VCF row.

## Phase 6: Query And Slice Tools

- [ ] Add `clinpatch query`.
  - `--variation-id`
  - `--allele-id`
  - `--region chr:start-end`
  - `--gene`
  - `--clinical-significance`
  - `--review-status`
  - `--condition`
  - `--disease-db`
- [ ] Add `clinpatch slice`.
  - Output VCF, JSONL, CSV, or SQLite.
  - Support filters by gene, significance, review status, disease, chromosome, Variation ID list, and genomic interval.
  - Support `--limit`, `--fields`, and `--include-raw`.
- [ ] Add `clinpatch stats`.
  - Counts by clinical significance.
  - Counts by review status.
  - Counts by gene.
  - Counts by variant type.
  - Counts by chromosome.
- [ ] Add `clinpatch ids`.
  - Accept a file of Variation IDs or Allele IDs.
  - Emit matching rows from VCF, SQLite, or static chunk bundle.
- [ ] Add `clinpatch explain-key`.
  - Show how a VCF row maps to the stable patch key and chunk path.

## Phase 7: GitHub Actions

- [ ] Add CI workflow `.github/workflows/ci.yml`.
  - `cargo fmt --check`
  - `cargo clippy -- -D warnings`
  - `cargo test`
  - Build release binary.
- [ ] Add update workflow `.github/workflows/update-clinvar.yml`.
  - Trigger manually with `workflow_dispatch`.
  - Trigger on schedule, likely weekly after NCBI publishes.
  - Run `clinpatch latest`.
  - Exit without changes if no new release exists.
  - Fetch latest GRCh38.
  - Generate patch from previous known release.
  - Generate chunks and indexes.
  - Verify patch and chunk manifests.
  - Commit generated small files back to `main` using the GitHub Actions bot.
  - Upload large chunk/patch bundles to GitHub Releases when they are too large for normal Git.
- [ ] Add Pages workflow only for the demo/static explorer.
  - Publish `public/` and small static manifests.
  - Do not publish full data through Pages unless size stays within limits.
- [ ] Add release workflow.
  - Build macOS/Linux binaries.
  - Attach binaries to GitHub Release.
  - Attach generated ClinVar data bundle when appropriate.

## Phase 8: Web Demo

- [ ] Port current `public/` demo.
- [ ] Update demo to read the new root static manifest.
- [ ] Let user choose release.
- [ ] Let user search by:
  - Variation ID.
  - Chromosome and position.
  - Row range.
  - Gene/significance later.
- [ ] Ensure all data fetches use normal static HTTP GET and `Range` requests.
- [ ] Add a small fixture for CI and Pages preview.

## Phase 9: Tests

- [ ] Unit tests for VCF parsing.
- [ ] Unit tests for INFO parsing and preservation.
- [ ] Unit tests for stable key generation.
- [ ] Unit tests for patch record serialization.
- [ ] Integration test with tiny two-release fixture.
- [ ] Integration test for:
  - Fetch fixture.
  - Diff.
  - Apply patch.
  - Build chunks.
  - Query by ID.
  - Query by row range.
  - Query by position.
- [ ] Golden tests for static manifest format.
- [ ] CI smoke test against a small committed fixture, not the full ClinVar release.

## First Working Slice

This is the shortest path to something useful on GitHub:

1. Port the Rust prototype and demo.
2. Add `.gitignore`, README, and CI.
3. Add `fetch`, `latest`, and release manifest support.
4. Add a manual GitHub Action that fetches GRCh38 and detects updates.
5. Generate patch manifest and `changes.jsonl.gz` for latest-vs-previous.
6. Generate chromosome/coordinate chunks plus row/id/position indexes.
7. Commit small manifests to repo.
8. Upload large generated files as GitHub Release assets.
9. Publish the demo with GitHub Pages using small fixture data first.

## Open Decisions

- [ ] Binary name: `clinpatch`, `clinvar-tool`, or both with an alias.
- [ ] Canonical committed data path layout.
- [ ] Whether patch files themselves should be committed when below 100 MiB or always uploaded as release assets.
- [ ] Initial coordinate bin size or target shard byte size.
- [ ] Whether the first static chunks should be plain VCF, BGZF, or both.
- [ ] Whether to add Git LFS, or avoid it and use GitHub Releases/object storage instead.
- [ ] Whether GitHub Pages is only a demo host or also a static data host.

## References

- NCBI ClinVar GRCh38 VCF directory: `https://ftp.ncbi.nlm.nih.gov/pub/clinvar/vcf_GRCh38/`
- GitHub large file limits: `https://docs.github.com/en/repositories/working-with-files/managing-large-files/about-large-files-on-github`
- GitHub Pages limits: `https://docs.github.com/en/pages/getting-started-with-github-pages/github-pages-limits`
