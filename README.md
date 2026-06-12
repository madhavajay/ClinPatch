# ClinPatch

Rust CLI for generating ClinVar release patches and static HTTP range-query files.

The first working target is GRCh38 ClinVar split into GitHub-sized chromosome/coordinate chunks. Each chunk is a plain VCF with sidecar byte indexes, so a static host only needs ordinary `GET`, `HEAD`, and `Range` support.

## Build Locally

```sh
cargo build --release
```

The local binary is:

```sh
./target/release/clinpatch
```

## Build Coordinate Chunks

This command splits a VCF into coordinate-bin chunks and creates row, Variation ID, and position indexes for every chunk:

```sh
./target/release/clinpatch chunks build \
  public/clinvar.GRCh38.sample.vcf \
  --out tmp/chunks-demo \
  --assembly GRCh38 \
  --bin-size 1000000 \
  --max-bytes 1000000 \
  --row-stride 128
```

The output layout is:

```text
tmp/chunks-demo/
  manifest.json
  chunks/
    1/
      000000000001-000001000000.vcf
      000000000001-000001000000.vcf.rows.json
      000000000001-000001000000.vcf.ids.json
      000000000001-000001000000.vcf.positions.json
```

For full ClinVar, keep `--max-bytes` below GitHub's normal-file warning threshold when files will be committed. The default is 45 MiB.

## Serve Locally

```sh
./target/release/clinpatch serve --root tmp/chunks-demo --bind 127.0.0.1:8765
```

## Example HTTP Range Queries With Curl

Fetch the root manifest:

```sh
curl http://127.0.0.1:8765/manifest.json
```

Pick the first chunk data file from the manifest:

```sh
DATA_FILE="$(curl -s http://127.0.0.1:8765/manifest.json | jq -r '.chunks[0].data_file')"
ROWS_INDEX="$(curl -s http://127.0.0.1:8765/manifest.json | jq -r '.chunks[0].rows_index')"
IDS_INDEX="$(curl -s http://127.0.0.1:8765/manifest.json | jq -r '.chunks[0].ids_index')"
POSITIONS_INDEX="$(curl -s http://127.0.0.1:8765/manifest.json | jq -r '.chunks[0].positions_index')"
```

Fetch the first data row in that chunk by byte range:

```sh
START="$(curl -s "http://127.0.0.1:8765/$ROWS_INDEX" | jq -r '.checkpoints[0].offset')"
NEXT="$(curl -s "http://127.0.0.1:8765/$ROWS_INDEX" | jq -r '.checkpoints[1].offset')"
END="$((NEXT - 1))"

curl -H "Range: bytes=$START-$END" \
  "http://127.0.0.1:8765/$DATA_FILE"
```

Fetch one exact ClinVar Variation ID from the ID index:

```sh
ID_RECORD="$(curl -s "http://127.0.0.1:8765/$IDS_INDEX" | jq -r '.records[0]')"
ID_START="$(printf '%s' "$ID_RECORD" | jq -r '.offset')"
ID_LENGTH="$(printf '%s' "$ID_RECORD" | jq -r '.length')"
ID_END="$((ID_START + ID_LENGTH - 1))"

curl -H "Range: bytes=$ID_START-$ID_END" \
  "http://127.0.0.1:8765/$DATA_FILE"
```

Fetch all records at the first indexed chromosome/position:

```sh
POS_RECORD="$(curl -s "http://127.0.0.1:8765/$POSITIONS_INDEX" | jq -r '.positions[0].records[0]')"
POS_START="$(printf '%s' "$POS_RECORD" | jq -r '.offset')"
POS_LENGTH="$(printf '%s' "$POS_RECORD" | jq -r '.length')"
POS_END="$((POS_START + POS_LENGTH - 1))"

curl -H "Range: bytes=$POS_START-$POS_END" \
  "http://127.0.0.1:8765/$DATA_FILE"
```

## Patch Streams

The standalone release patch path is separate from hosted shard replacement. The prototype command still emits a full `changes.jsonl.gz` patch stream:

```sh
./target/release/clinpatch diff \
  --old-release 2026-04-26 \
  --new-release 2026-05-23 \
  --old 2026-04-26/clinvar.vcf.gz \
  --new 2026-05-23/clinvar.GRCh38.vcf.gz \
  --out data/patches/GRCh38/2026-04-26_to_2026-05-23
```

The intended model is:

- Static hosted files use chunk manifests and replace only changed chunk files.
- Local users can fetch standalone `OLD_to_NEW/changes.jsonl.gz` patch files and apply them to their local copy.
