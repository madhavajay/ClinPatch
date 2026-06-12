#!/usr/bin/env python3
import json
import os
import signal
import sys
import urllib.error
import urllib.request

signal.signal(signal.SIGPIPE, signal.SIG_DFL)


RAW_BASE = os.environ.get(
    "RAW_BASE",
    "https://raw.githubusercontent.com/madhavajay/ClinPatch/main/public/chunks-brca1",
).rstrip("/")

# BRCA1 from GENCODE v50 GRCh38:
# BRCA1 ENSG00000012048.28 chr17:43044292-43170245 (-)
CHROM = os.environ.get("CHROM", "17")
START_POS = int(os.environ.get("START_POS", "43044292"))
END_POS = int(os.environ.get("END_POS", "43170245"))


def fetch_json(url: str):
    with urllib.request.urlopen(url) as response:
        return json.load(response)


def fetch_range(url: str, start: int, end: int) -> str:
    request = urllib.request.Request(url, headers={"Range": f"bytes={start}-{end}"})
    with urllib.request.urlopen(request) as response:
        return response.read().decode("utf-8")


def main() -> int:
    manifest = fetch_json(f"{RAW_BASE}/manifest.json")
    chunks = [
        chunk
        for chunk in manifest["chunks"]
        if chunk["chrom"] == CHROM
        and chunk["start"] <= END_POS
        and chunk["end"] >= START_POS
    ]

    if not chunks:
        print(
            f"No hosted ClinVar chunk overlaps BRCA1 GRCh38 {CHROM}:{START_POS}-{END_POS}.",
            file=sys.stderr,
        )
        return 0

    for chunk in chunks:
        data_url = f"{RAW_BASE}/{chunk['data_file']}"
        positions = fetch_json(f"{RAW_BASE}/{chunk['positions_index']}")
        for position in positions["positions"]:
            if START_POS <= position["pos"] <= END_POS:
                for record in position["records"]:
                    start = int(record["offset"])
                    end = start + int(record["length"]) - 1
                    sys.stdout.write(fetch_range(data_url, start, end))

    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except BrokenPipeError:
        raise SystemExit(0)
    except urllib.error.URLError as error:
        print(f"request failed: {error}", file=sys.stderr)
        raise SystemExit(1)
