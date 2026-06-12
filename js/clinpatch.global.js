(function (global) {
  "use strict";

  const DEFAULT_RAW_BASE =
    "https://raw.githubusercontent.com/madhavajay/ClinPatch/main/public/chunks-brca1";
  const DEFAULT_GENE_INDEX =
    "https://raw.githubusercontent.com/madhavajay/ClinPatch/main/public/genes/gencode.v50.GRCh38.genes.json";
  const DEFAULT_ASSEMBLY = "GRCh38";

  class ClinPatchClient {
    constructor(options = {}) {
      this.assembly = options.assembly || DEFAULT_ASSEMBLY;
      this.rawBase = (options.rawBase || DEFAULT_RAW_BASE).replace(/\/$/, "");
      this.geneIndexUrl = options.geneIndexUrl || DEFAULT_GENE_INDEX;
      this.fetch = options.fetch || global.fetch;
      if (!this.fetch) {
        throw new Error("ClinPatchClient requires fetch");
      }
      this._manifest = null;
      this._geneIndex = null;
      this._jsonCache = new Map();
    }

    async manifest() {
      if (!this._manifest) {
        this._manifest = await this._fetchJson(`${this.rawBase}/manifest.json`);
        if (this._manifest.assembly && this._manifest.assembly !== this.assembly) {
          throw new Error(
            `manifest assembly ${this._manifest.assembly} does not match requested assembly ${this.assembly}`,
          );
        }
      }
      return this._manifest;
    }

    async geneIndex() {
      if (!this._geneIndex) {
        this._geneIndex = await this._fetchJson(this.geneIndexUrl);
      }
      return this._geneIndex;
    }

    async resolveGene(symbol) {
      const wanted = symbol.toUpperCase();
      const index = await this.geneIndex();
      const record = index.genes.find((gene) => gene.symbol_norm === wanted);
      if (!record) {
        throw new Error(`gene not found in index: ${symbol}`);
      }
      return {
        symbol: record.symbol,
        geneId: record.gene_id,
        geneType: record.gene_type,
        chrom: record.chrom,
        start: record.start,
        end: record.end,
        strand: record.strand,
      };
    }

    async *queryGene(symbol, options = {}) {
      const gene = await this.resolveGene(symbol);
      const start = options.start ?? gene.start;
      const end = options.end ?? gene.end;
      yield* this.queryRegion({ chrom: gene.chrom, start, end }, options);
    }

    async *queryRegion(region, options = {}) {
      const parsed = typeof region === "string" ? parseRegion(region) : normalizeRegion(region);
      const format = options.format || "vcf";
      const filter = options.filter || (() => true);
      let emitted = 0;

      for await (const row of this._regionRows(parsed)) {
        const value = format === "json" || format === "jsonl" ? parseVcfRow(row) : row;
        if (!filter(value)) {
          continue;
        }
        yield value;
        emitted += 1;
        if (options.limit && emitted >= options.limit) {
          return;
        }
      }
    }

    async collectGene(symbol, options = {}) {
      const records = [];
      for await (const record of this.queryGene(symbol, options)) {
        records.push(record);
      }
      return records;
    }

    async collectRegion(region, options = {}) {
      const records = [];
      for await (const record of this.queryRegion(region, options)) {
        records.push(record);
      }
      return records;
    }

    async *_regionRows(region) {
      const manifest = await this.manifest();
      const chunks = manifest.chunks.filter(
        (chunk) => chunk.chrom === region.chrom && chunk.start <= region.end && chunk.end >= region.start,
      );

      for (const chunk of chunks) {
        const dataUrl = `${this.rawBase}/${chunk.data_file}`;
        const index = await this._fetchJson(`${this.rawBase}/${chunk.positions_index}`);
        for (const position of index.positions) {
          if (position.pos < region.start || position.pos > region.end) {
            continue;
          }
          for (const record of position.records) {
            const offset = Number(record.offset);
            const end = offset + Number(record.length) - 1;
            yield await this._fetchTextRange(dataUrl, offset, end);
          }
        }
      }
    }

    async _fetchJson(url) {
      if (!this._jsonCache.has(url)) {
        this._jsonCache.set(
          url,
          this.fetch(url).then(async (response) => {
            if (!response.ok) {
              throw new Error(`${url} returned HTTP ${response.status}`);
            }
            return response.json();
          }),
        );
      }
      return this._jsonCache.get(url);
    }

    async _fetchTextRange(url, start, end) {
      const response = await this.fetch(url, {
        headers: {
          Range: `bytes=${start}-${end}`,
        },
      });
      if (!response.ok && response.status !== 206) {
        throw new Error(`${url} returned HTTP ${response.status}`);
      }
      return response.text();
    }
  }

  function parseRegion(region) {
    const match = /^([^:]+):([0-9,]+)-([0-9,]+)$/.exec(region);
    if (!match) {
      throw new Error(`invalid region ${region}; expected CHROM:START-END`);
    }
    return normalizeRegion({
      chrom: match[1],
      start: Number.parseInt(match[2].replaceAll(",", ""), 10),
      end: Number.parseInt(match[3].replaceAll(",", ""), 10),
    });
  }

  function normalizeRegion(region) {
    let chrom = String(region.chrom).replace(/^chr/, "");
    if (chrom === "M") {
      chrom = "MT";
    }
    const start = Number(region.start);
    const end = Number(region.end);
    if (!chrom || !Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start < 1 || start > end) {
      throw new Error("invalid region coordinates");
    }
    return { chrom, start, end };
  }

  function parseVcfRow(row) {
    const columns = row.trimEnd().split("\t");
    return {
      chrom: columns[0],
      pos: Number.parseInt(columns[1], 10),
      id: columns[2],
      ref: columns[3],
      alt: columns[4],
      qual: columns[5],
      filter: columns[6],
      info: parseInfo(columns[7] || ""),
      raw: row.trimEnd(),
    };
  }

  function parseInfo(info) {
    const values = {};
    for (const field of info.split(";")) {
      if (!field) {
        continue;
      }
      const separator = field.indexOf("=");
      if (separator === -1) {
        values[field] = true;
      } else {
        values[field.slice(0, separator)] = field.slice(separator + 1);
      }
    }
    return values;
  }

  global.ClinPatch = {
    ClinPatchClient,
    parseRegion,
    normalizeRegion,
    parseVcfRow,
    parseInfo,
    DEFAULT_RAW_BASE,
    DEFAULT_GENE_INDEX,
    DEFAULT_ASSEMBLY,
  };
})(globalThis);
