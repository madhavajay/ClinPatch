import argparse
import json
import os
import signal
import sys
import urllib.request
from urllib.parse import urlparse


DEFAULT_RAW_BASE = "https://raw.githubusercontent.com/madhavajay/ClinPatch/main/public/chunks-brca1"
DEFAULT_GENE_INDEX = (
    "https://raw.githubusercontent.com/madhavajay/ClinPatch/main/public/genes/"
    "gencode.v50.GRCh38.genes.json"
)

signal.signal(signal.SIGPIPE, signal.SIG_DFL)


def read_url(url: str, start: int | None = None, end: int | None = None) -> bytes:
    parsed = urlparse(url)
    if parsed.scheme == "file":
        path = urllib.request.url2pathname(parsed.path)
        with open(path, "rb") as file:
            if start is not None:
                file.seek(start)
                return file.read(end - start + 1)
            return file.read()

    headers = {}
    if start is not None:
        headers["Range"] = f"bytes={start}-{end}"
    request = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(request) as response:
        return response.read()


def read_json(url: str):
    return json.loads(read_url(url).decode("utf-8"))


def parse_region(region: str) -> tuple[str, int, int]:
    chrom, _, coords = region.partition(":")
    start, _, end = coords.partition("-")
    if not chrom or not start or not end:
        raise SystemExit(f"invalid --region {region!r}; expected CHROM:START-END")
    chrom = chrom.removeprefix("chr")
    if chrom == "M":
        chrom = "MT"
    return chrom, int(start.replace(",", "")), int(end.replace(",", ""))


def info_dict(info: str) -> dict[str, str | bool]:
    values: dict[str, str | bool] = {}
    for field in info.split(";"):
        if not field:
            continue
        if "=" in field:
            key, value = field.split("=", 1)
            values[key] = value
        else:
            values[field] = True
    return values


def vcf_to_json(line: str) -> dict[str, object]:
    columns = line.rstrip("\n").split("\t")
    return {
        "chrom": columns[0],
        "pos": int(columns[1]),
        "id": columns[2],
        "ref": columns[3],
        "alt": columns[4],
        "qual": columns[5],
        "filter": columns[6],
        "info": info_dict(columns[7]) if len(columns) > 7 else {},
        "raw": line.rstrip("\n"),
    }


def resolve_gene(gene_index_url: str, gene: str) -> tuple[str, int, int]:
    wanted = gene.upper()
    index = read_json(gene_index_url)
    for record in index["genes"]:
        if record["symbol_norm"] == wanted:
            return record["chrom"], int(record["start"]), int(record["end"])
    raise SystemExit(f"gene not found in index: {gene}")


def iter_records(raw_base: str, chrom: str, start: int, end: int):
    manifest = read_json(f"{raw_base}/manifest.json")
    chunks = [
        chunk
        for chunk in manifest["chunks"]
        if chunk["chrom"] == chrom and chunk["start"] <= end and chunk["end"] >= start
    ]
    for chunk in chunks:
        data_url = f"{raw_base}/{chunk['data_file']}"
        positions = read_json(f"{raw_base}/{chunk['positions_index']}")
        for position in positions["positions"]:
            if start <= position["pos"] <= end:
                for record in position["records"]:
                    offset = int(record["offset"])
                    length = int(record["length"])
                    yield read_url(data_url, offset, offset + length - 1).decode("utf-8")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Fetch ClinVar rows from static ClinPatch chunks."
    )
    target = parser.add_mutually_exclusive_group(required=True)
    target.add_argument("--gene", help="Gene symbol, resolved through the gene index.")
    target.add_argument("--region", help="Genomic interval, for example 17:43044293-43045642.")
    parser.add_argument("--format", choices=["vcf", "jsonl"], default="vcf")
    parser.add_argument("--limit", type=int, default=0, help="Maximum records to print.")
    parser.add_argument("--raw-base", default=os.environ.get("RAW_BASE", DEFAULT_RAW_BASE))
    parser.add_argument(
        "--gene-index", default=os.environ.get("GENE_INDEX_URL", DEFAULT_GENE_INDEX)
    )
    args = parser.parse_args(argv)

    if args.gene:
        chrom, start, end = resolve_gene(args.gene_index, args.gene)
    else:
        chrom, start, end = parse_region(args.region)

    count = 0
    for row in iter_records(args.raw_base.rstrip("/"), chrom, start, end):
        if args.format == "jsonl":
            print(json.dumps(vcf_to_json(row), separators=(",", ":")))
        else:
            sys.stdout.write(row)
        count += 1
        if args.limit and count >= args.limit:
            break

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
