const DATA_URL = "./clinvar.GRCh38.sample.vcf";
const ROW_INDEX_URL = "./clinvar.GRCh38.sample.vcf.rows.json";
const ID_INDEX_URL = "./clinvar.GRCh38.sample.vcf.ids.json";
const POSITION_INDEX_URL = "./clinvar.GRCh38.sample.vcf.positions.json";

let rowIndex = null;
let idIndex = null;
let positionIndex = null;
let idLookup = new Map();
let positionLookup = new Map();

const els = {
  dataFile: document.querySelector("#data-file"),
  rowCount: document.querySelector("#row-count"),
  idCount: document.querySelector("#id-count"),
  positionCount: document.querySelector("#position-count"),
  rowStride: document.querySelector("#row-stride"),
  message: document.querySelector("#message"),
  result: document.querySelector("#result"),
  rangeUsed: document.querySelector("#range-used"),
  reload: document.querySelector("#reload-indexes"),
  positionForm: document.querySelector("#position-search"),
  chrom: document.querySelector("#chrom"),
  position: document.querySelector("#position"),
  idForm: document.querySelector("#id-search"),
  idInput: document.querySelector("#variation-id"),
  rowForm: document.querySelector("#row-search"),
  rowStart: document.querySelector("#row-start"),
  rowCountInput: document.querySelector("#row-count-input"),
};

async function loadJson(url) {
  const response = await fetch(url, { cache: "no-store" });
  if (!response.ok) {
    throw new Error(`${url} returned HTTP ${response.status}`);
  }
  return response.json();
}

async function loadIndexes() {
  setMessage("Loading row, ID, and position indexes...");
  [rowIndex, idIndex, positionIndex] = await Promise.all([
    loadJson(ROW_INDEX_URL),
    loadJson(ID_INDEX_URL),
    loadJson(POSITION_INDEX_URL),
  ]);

  idLookup = new Map(idIndex.records.map((record) => [record.id, record]));
  positionLookup = new Map(
    positionIndex.positions.map((entry) => [positionKey(entry.chrom, entry.pos), entry])
  );
  els.dataFile.textContent = rowIndex.data_file;
  els.rowCount.textContent = rowIndex.row_count.toLocaleString();
  els.idCount.textContent = idIndex.records.length.toLocaleString();
  els.positionCount.textContent = positionIndex.positions.length.toLocaleString();
  els.rowStride.textContent = rowIndex.stride.toLocaleString();
  setMessage("");

  const example = idIndex.records.find((record) => record.id === "1168967") || idIndex.records[0];
  if (example) {
    els.idInput.value = example.id;
  }
}

function positionKey(chrom, pos) {
  return `${String(chrom).trim()}:${Number.parseInt(pos, 10)}`;
}

function setMessage(message) {
  els.message.textContent = message;
}

function setRange(start, end) {
  els.rangeUsed.textContent = `Range: bytes=${start}-${end}`;
}

async function fetchByteRange(start, end) {
  const response = await fetch(DATA_URL, {
    headers: {
      Range: `bytes=${start}-${end}`,
    },
  });
  if (response.status !== 206) {
    throw new Error(`Expected HTTP 206 Partial Content, got HTTP ${response.status}`);
  }
  setRange(start, end);
  return response.text();
}

function parseRow(line) {
  const fields = line.trimEnd().split("\t");
  return {
    chrom: fields[0] || "",
    pos: fields[1] || "",
    id: fields[2] || "",
    ref: fields[3] || "",
    alt: fields[4] || "",
    info: fields[7] || "",
    raw: line.trimEnd(),
  };
}

function formatRecord(line) {
  const record = parseRow(line);
  const info = Object.fromEntries(
    record.info
      .split(";")
      .map((part) => part.split("="))
      .filter((pair) => pair.length === 2)
  );

  return [
    `Variation ID: ${record.id}`,
    `Location: ${record.chrom}:${record.pos}`,
    `Allele: ${record.ref} > ${record.alt}`,
    `Allele ID: ${info.ALLELEID || "-"}`,
    `Gene: ${info.GENEINFO || "-"}`,
    `Clinical significance: ${info.CLNSIG || "-"}`,
    `Review status: ${info.CLNREVSTAT || "-"}`,
    `Condition: ${info.CLNDN || "-"}`,
    "",
    "Raw VCF row:",
    record.raw,
  ].join("\n");
}

function checkpointForRow(row) {
  for (let index = rowIndex.checkpoints.length - 1; index >= 0; index -= 1) {
    const checkpoint = rowIndex.checkpoints[index];
    if (checkpoint.row <= row) {
      return checkpoint;
    }
  }
  return rowIndex.checkpoints[0];
}

function checkpointAfterRow(row) {
  return rowIndex.checkpoints.find((checkpoint) => checkpoint.row > row);
}

async function searchById(event) {
  event.preventDefault();
  setMessage("");
  const id = els.idInput.value.trim();
  if (!id) {
    setMessage("Enter a ClinVar Variation ID.");
    return;
  }

  const entry = idLookup.get(id);
  if (!entry) {
    setMessage(`ID ${id} is not in this sample index.`);
    els.result.textContent = "";
    els.rangeUsed.textContent = "No matching byte range";
    return;
  }

  const start = entry.offset;
  const end = entry.offset + entry.length - 1;
  const text = await fetchByteRange(start, end);
  els.result.textContent = formatRecord(text);
}

async function searchByPosition(event) {
  event.preventDefault();
  setMessage("");

  const chrom = els.chrom.value.trim();
  const pos = Number.parseInt(els.position.value, 10);
  if (!chrom || !Number.isSafeInteger(pos) || pos < 1) {
    setMessage("Enter a chromosome and positive position.");
    return;
  }

  const entry = positionLookup.get(positionKey(chrom, pos));
  if (!entry) {
    setMessage(`No sample records found at ${chrom}:${pos}.`);
    els.result.textContent = "";
    els.rangeUsed.textContent = "No matching byte range";
    return;
  }

  const records = [];
  const ranges = [];
  for (const record of entry.records) {
    const start = record.offset;
    const end = record.offset + record.length - 1;
    ranges.push(`${start}-${end}`);
    records.push(await fetchByteRange(start, end));
  }

  els.rangeUsed.textContent = `Range${ranges.length === 1 ? "" : "s"}: bytes=${ranges.join(", ")}`;
  els.result.textContent = records.map((line) => formatRecord(line)).join("\n\n---\n\n");
}

async function fetchRows(event) {
  event.preventDefault();
  setMessage("");

  const startRow = Number.parseInt(els.rowStart.value, 10);
  const count = Number.parseInt(els.rowCountInput.value, 10);
  if (!Number.isSafeInteger(startRow) || startRow < 1 || !Number.isSafeInteger(count) || count < 1) {
    setMessage("Enter a valid 1-based start row and positive count.");
    return;
  }

  const endRow = startRow + count - 1;
  if (endRow > rowIndex.row_count) {
    setMessage(`Requested row ${endRow}, but this sample only has ${rowIndex.row_count} rows.`);
    return;
  }

  const startCheckpoint = checkpointForRow(startRow);
  const endCheckpoint = checkpointAfterRow(endRow);
  const rangeStart = startCheckpoint.offset;
  const rangeEnd = (endCheckpoint ? endCheckpoint.offset : rowIndex.file_size) - 1;
  const text = await fetchByteRange(rangeStart, rangeEnd);
  const lines = text.trimEnd().split("\n");
  const skip = startRow - startCheckpoint.row;
  const selected = lines.slice(skip, skip + count);
  els.result.textContent = selected.map((line) => formatRecord(line)).join("\n\n---\n\n");
}

els.reload.addEventListener("click", () => {
  loadIndexes().catch((error) => setMessage(error.message));
});
els.positionForm.addEventListener("submit", (event) => {
  searchByPosition(event).catch((error) => setMessage(error.message));
});
els.idForm.addEventListener("submit", (event) => {
  searchById(event).catch((error) => setMessage(error.message));
});
els.rowForm.addEventListener("submit", (event) => {
  fetchRows(event).catch((error) => setMessage(error.message));
});

loadIndexes().catch((error) => setMessage(error.message));
