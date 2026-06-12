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
DATA_FILE="$(curl -fsSL http://127.0.0.1:8765/manifest.json | jq -r '.chunks[0].data_file')"
ROWS_INDEX="$(curl -fsSL http://127.0.0.1:8765/manifest.json | jq -r '.chunks[0].rows_index')"
IDS_INDEX="$(curl -fsSL http://127.0.0.1:8765/manifest.json | jq -r '.chunks[0].ids_index')"
POSITIONS_INDEX="$(curl -fsSL http://127.0.0.1:8765/manifest.json | jq -r '.chunks[0].positions_index')"
```

Fetch the first data row in that chunk by byte range:

```sh
START="$(curl -fsSL "http://127.0.0.1:8765/$ROWS_INDEX" | jq -r '.checkpoints[0].offset')"
NEXT="$(curl -fsSL "http://127.0.0.1:8765/$ROWS_INDEX" | jq -r '.checkpoints[1].offset')"
END="$((NEXT - 1))"

curl -H "Range: bytes=$START-$END" \
  "http://127.0.0.1:8765/$DATA_FILE"
```

Fetch one exact ClinVar Variation ID from the ID index:

```sh
ID_RECORD="$(curl -fsSL "http://127.0.0.1:8765/$IDS_INDEX" | jq -r '.records[0]')"
ID_START="$(printf '%s' "$ID_RECORD" | jq -r '.offset')"
ID_LENGTH="$(printf '%s' "$ID_RECORD" | jq -r '.length')"
ID_END="$((ID_START + ID_LENGTH - 1))"

curl -H "Range: bytes=$ID_START-$ID_END" \
  "http://127.0.0.1:8765/$DATA_FILE"
```

Fetch all records at the first indexed chromosome/position:

```sh
POS_RECORD="$(curl -fsSL "http://127.0.0.1:8765/$POSITIONS_INDEX" | jq -r '.positions[0].records[0]')"
POS_START="$(printf '%s' "$POS_RECORD" | jq -r '.offset')"
POS_LENGTH="$(printf '%s' "$POS_RECORD" | jq -r '.length')"
POS_END="$((POS_START + POS_LENGTH - 1))"

curl -H "Range: bytes=$POS_START-$POS_END" \
  "http://127.0.0.1:8765/$DATA_FILE"
```

## Raw GitHub Range Query Example

This example does not use the Rust tool or a local server. It curls the manifest from raw GitHub, finds chunks overlapping a chromosome interval, curls the matching position indexes from raw GitHub, then curls byte ranges from the raw GitHub chunk VCF files.

```sh
RAW_BASE="https://raw.githubusercontent.com/madhavajay/ClinPatch/main/public/chunks-brca1"
CHROM="1"
START_POS=1041000
END_POS=1054000

MANIFEST="$(curl -fsSL "$RAW_BASE/manifest.json")"

printf '%s' "$MANIFEST" |
jq -c --arg chrom "$CHROM" --argjson start "$START_POS" --argjson end "$END_POS" '
  .chunks[]
  | select(.chrom == $chrom and .start <= $end and .end >= $start)
' |
while read -r CHUNK; do
  DATA_FILE="$(printf '%s' "$CHUNK" | jq -r '.data_file')"
  POS_INDEX="$(printf '%s' "$CHUNK" | jq -r '.positions_index')"

  curl -fsSL "$RAW_BASE/$POS_INDEX" |
  jq -c --argjson start "$START_POS" --argjson end "$END_POS" '
    .positions[]
    | select(.pos >= $start and .pos <= $end)
    | .records[]
  ' |
  while read -r REC; do
    OFFSET="$(printf '%s' "$REC" | jq -r '.offset')"
    LENGTH="$(printf '%s' "$REC" | jq -r '.length')"
    END_BYTE="$((OFFSET + LENGTH - 1))"

    curl -fsSL --range "$OFFSET-$END_BYTE" "$RAW_BASE/$DATA_FILE"
  done
done
```

That interval returns real ClinVar VCF rows from the committed GRCh38 sample chunk files.

## Raw GitHub BRCA1 Query Example

This example resolves a gene symbol through a static GENCODE v50 GRCh38 gene index, then uses simple coordinate overlap to fetch ClinVar rows from raw GitHub.

The easiest way to run this demo is:

```sh
./demo.sh
```

The gene mapping is:

```text
variant.chrom = gene.chrom
variant.pos BETWEEN gene.start AND gene.end
```

It uses GENCODE's GRCh38 coordinates, not gnomAD-derived gene mapping.

```sh
RAW_BASE="https://raw.githubusercontent.com/madhavajay/ClinPatch/main/public/chunks-brca1"
GENE_INDEX_URL="https://raw.githubusercontent.com/madhavajay/ClinPatch/main/public/genes/gencode.v50.GRCh38.genes.json"
GENE="AGRN"

GENE_RECORD="$(curl -fsSL "$GENE_INDEX_URL" |
  jq -c --arg gene "$GENE" '.genes[] | select(.symbol_norm == ($gene | ascii_upcase))' |
  head -n 1)"

CHROM="$(printf '%s' "$GENE_RECORD" | jq -r '.chrom')"
START_POS="$(printf '%s' "$GENE_RECORD" | jq -r '.start')"
END_POS="$(printf '%s' "$GENE_RECORD" | jq -r '.end')"

MANIFEST="$(curl -fsSL "$RAW_BASE/manifest.json")"

printf '%s' "$MANIFEST" |
jq -c --arg chrom "$CHROM" --argjson start "$START_POS" --argjson end "$END_POS" '
  .chunks[]
  | select(.chrom == $chrom and .start <= $end and .end >= $start)
' |
while read -r CHUNK; do
  DATA_FILE="$(printf '%s' "$CHUNK" | jq -r '.data_file')"
  POS_INDEX="$(printf '%s' "$CHUNK" | jq -r '.positions_index')"

  curl -fsSL "$RAW_BASE/$POS_INDEX" |
  jq -c --argjson start "$START_POS" --argjson end "$END_POS" '
    .positions[]
    | select(.pos >= $start and .pos <= $end)
    | .records[]
  ' |
  while read -r REC; do
    OFFSET="$(printf '%s' "$REC" | jq -r '.offset')"
    LENGTH="$(printf '%s' "$REC" | jq -r '.length')"
    END_BYTE="$((OFFSET + LENGTH - 1))"

    curl -fsSL --range "$OFFSET-$END_BYTE" "$RAW_BASE/$DATA_FILE"
  done
done
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
