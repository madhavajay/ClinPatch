#!/usr/bin/env node

import { ClinPatchClient } from "../js/clinpatch.js";

function parseArgs(argv) {
  const args = {
    format: "vcf",
    limit: 0,
    rawBase: process.env.RAW_BASE,
    geneIndexUrl: process.env.GENE_INDEX_URL,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const next = () => {
      index += 1;
      if (index >= argv.length) throw new Error(`missing value for ${arg}`);
      return argv[index];
    };
    if (arg === "--gene") args.gene = next();
    else if (arg === "--region") args.region = next();
    else if (arg === "--format") args.format = next();
    else if (arg === "--limit") args.limit = Number.parseInt(next(), 10);
    else if (arg === "--raw-base") args.rawBase = next();
    else if (arg === "--gene-index") args.geneIndexUrl = next();
    else if (arg === "-h" || arg === "--help") args.help = true;
    else throw new Error(`unknown argument: ${arg}`);
  }
  return args;
}

function usage() {
  return `Usage:
  clinpatch-query --gene BRCA1 [--format vcf|jsonl] [--limit N]
  clinpatch-query --region 17:43044293-43045642 [--format vcf|jsonl] [--limit N]

Options:
  --raw-base URL      Root URL containing manifest.json and chunks
  --gene-index URL    Gene coordinate index URL
`;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    process.stdout.write(usage());
    return;
  }
  if (!args.gene && !args.region) throw new Error("pass --gene or --region");
  if (args.gene && args.region) throw new Error("pass only one of --gene or --region");
  if (!["vcf", "jsonl"].includes(args.format)) throw new Error("--format must be vcf or jsonl");

  const client = new ClinPatchClient({
    rawBase: args.rawBase,
    geneIndexUrl: args.geneIndexUrl,
  });
  const options = {
    format: args.format === "jsonl" ? "json" : "vcf",
    limit: args.limit,
  };
  const records = args.gene
    ? client.queryGene(args.gene, options)
    : client.queryRegion(args.region, options);

  for await (const record of records) {
    if (args.format === "jsonl") {
      process.stdout.write(`${JSON.stringify(record)}\n`);
    } else {
      process.stdout.write(record);
    }
  }
}

main().catch((error) => {
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 1;
});
