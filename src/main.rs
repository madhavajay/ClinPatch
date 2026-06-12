use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write, empty};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tiny_http::{Header, Method, Response, Server, StatusCode};

#[derive(Parser)]
#[command(name = "clinpatch")]
#[command(about = "Import ClinVar VCF files into SQLite and generate release patches.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Import {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        release: String,
        vcf: PathBuf,
        #[arg(long)]
        limit: Option<u64>,
    },
    Diff {
        #[arg(long)]
        old_release: String,
        #[arg(long)]
        new_release: String,
        #[arg(long)]
        old: PathBuf,
        #[arg(long)]
        new: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        limit: Option<u64>,
    },
    ApplyPatch {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        old_release: String,
        #[arg(long)]
        new_release: String,
        patch: PathBuf,
    },
    WritePlain {
        input: PathBuf,
        output: PathBuf,
        #[arg(long)]
        limit: Option<u64>,
    },
    IndexRows {
        file: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 128)]
        stride: u64,
    },
    IndexIds {
        file: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    IndexPositions {
        file: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    RowsLocal {
        file: PathBuf,
        #[arg(long)]
        index: PathBuf,
        #[arg(long)]
        start: u64,
        #[arg(long)]
        count: u64,
    },
    Chunks {
        #[command(subcommand)]
        command: ChunksCommand,
    },
    Genes {
        #[command(subcommand)]
        command: GenesCommand,
    },
    Serve {
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8000")]
        bind: String,
    },
}

#[derive(Subcommand)]
enum GenesCommand {
    Build {
        gtf: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "GRCh38")]
        assembly: String,
        #[arg(
            long,
            default_value = "https://ftp.ebi.ac.uk/pub/databases/gencode/Gencode_human/release_50/gencode.v50.annotation.gtf.gz"
        )]
        source_url: String,
    },
}

#[derive(Subcommand)]
enum ChunksCommand {
    Build {
        input: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "GRCh38")]
        assembly: String,
        #[arg(long, default_value_t = 10_000_000)]
        bin_size: u64,
        #[arg(long, default_value_t = 45 * 1024 * 1024)]
        max_bytes: u64,
        #[arg(long, default_value_t = 128)]
        row_stride: u64,
        #[arg(long)]
        limit: Option<u64>,
        #[arg(long)]
        region: Option<String>,
    },
}

#[derive(Debug, Clone)]
struct VcfRecord {
    chrom: String,
    pos: i64,
    variation_id: String,
    reference: String,
    alternate: String,
    qual: String,
    filter: String,
    info: String,
    allele_id: Option<String>,
    key: String,
    hash: String,
    raw_line: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PatchManifest {
    old_release: String,
    new_release: String,
    old_path: String,
    new_path: String,
    added: u64,
    changed: u64,
    removed: u64,
    unchanged: u64,
    old_records_scanned: u64,
    new_records_scanned: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op")]
enum PatchRecord {
    #[serde(rename = "added")]
    Added { key: String, new: PatchVcfRecord },
    #[serde(rename = "changed")]
    Changed {
        key: String,
        old_hash: String,
        new: PatchVcfRecord,
    },
    #[serde(rename = "removed")]
    Removed {
        key: String,
        old_hash: String,
        old_raw_line: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct PatchVcfRecord {
    hash: String,
    raw_line: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct RowIndex {
    format: String,
    version: u32,
    data_file: String,
    file_size: u64,
    file_hash_sha256: String,
    row_count: u64,
    header_bytes: u64,
    stride: u64,
    checkpoints: Vec<RowCheckpoint>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RowCheckpoint {
    row: u64,
    offset: u64,
    chrom: String,
    pos: Option<u64>,
    id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct IdIndex {
    format: String,
    version: u32,
    data_file: String,
    file_size: u64,
    file_hash_sha256: String,
    row_count: u64,
    records: Vec<IdIndexRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
struct IdIndexRecord {
    id: String,
    row: u64,
    offset: u64,
    length: u64,
    chrom: String,
    pos: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PositionIndex {
    format: String,
    version: u32,
    data_file: String,
    file_size: u64,
    file_hash_sha256: String,
    row_count: u64,
    positions: Vec<PositionIndexEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PositionIndexEntry {
    chrom: String,
    pos: u64,
    records: Vec<PositionIndexRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PositionIndexRecord {
    id: String,
    row: u64,
    offset: u64,
    length: u64,
    reference: String,
    alternate: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChunkManifest {
    format: String,
    version: u32,
    assembly: String,
    source_file: String,
    bin_size: u64,
    max_bytes: u64,
    row_stride: u64,
    chunk_count: u64,
    row_count: u64,
    chunks: Vec<ChunkManifestEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChunkManifestEntry {
    chrom: String,
    start: u64,
    end: u64,
    part: u32,
    data_file: String,
    rows_index: String,
    ids_index: String,
    positions_index: String,
    file_size: u64,
    row_count: u64,
    file_hash_sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeneIndex {
    format: String,
    version: u32,
    assembly: String,
    source_url: String,
    gene_count: u64,
    genes: Vec<GeneIndexEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeneIndexEntry {
    symbol: String,
    symbol_norm: String,
    gene_id: String,
    gene_type: String,
    chrom: String,
    start: u64,
    end: u64,
    strand: String,
}

struct ActiveChunk {
    chrom: String,
    start: u64,
    end: u64,
    part: u32,
    data_path: PathBuf,
    writer: BufWriter<File>,
    row_count: u64,
    estimated_bytes: u64,
}

struct RegionFilter {
    chrom: String,
    start: u64,
    end: u64,
}

struct ChunkBuildOptions<'a> {
    assembly: &'a str,
    bin_size: u64,
    max_bytes: u64,
    row_stride: u64,
    limit: Option<u64>,
    region: Option<&'a str>,
}

impl From<&VcfRecord> for PatchVcfRecord {
    fn from(record: &VcfRecord) -> Self {
        Self {
            hash: record.hash.clone(),
            raw_line: record.raw_line.clone(),
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Import {
            db,
            release,
            vcf,
            limit,
        } => import_release(&db, &release, &vcf, limit),
        Command::Diff {
            old_release,
            new_release,
            old,
            new,
            out,
            limit,
        } => diff_releases(&old_release, &new_release, &old, &new, &out, limit),
        Command::ApplyPatch {
            db,
            old_release,
            new_release,
            patch,
        } => apply_patch(&db, &old_release, &new_release, &patch),
        Command::WritePlain {
            input,
            output,
            limit,
        } => write_plain_vcf(&input, &output, limit),
        Command::IndexRows { file, out, stride } => index_rows(&file, &out, stride),
        Command::IndexIds { file, out } => index_ids(&file, &out),
        Command::IndexPositions { file, out } => index_positions(&file, &out),
        Command::RowsLocal {
            file,
            index,
            start,
            count,
        } => rows_local(&file, &index, start, count),
        Command::Chunks { command } => match command {
            ChunksCommand::Build {
                input,
                out,
                assembly,
                bin_size,
                max_bytes,
                row_stride,
                limit,
                region,
            } => build_chunks(
                &input,
                &out,
                ChunkBuildOptions {
                    assembly: &assembly,
                    bin_size,
                    max_bytes,
                    row_stride,
                    limit,
                    region: region.as_deref(),
                },
            ),
        },
        Command::Genes { command } => match command {
            GenesCommand::Build {
                gtf,
                out,
                assembly,
                source_url,
            } => build_gene_index(&gtf, &out, &assembly, &source_url),
        },
        Command::Serve { root, bind } => serve_static(&root, &bind),
    }
}

fn import_release(
    db_path: &Path,
    release: &str,
    vcf_path: &Path,
    limit: Option<u64>,
) -> Result<()> {
    let mut conn = Connection::open(db_path)
        .with_context(|| format!("opening sqlite database {}", db_path.display()))?;
    setup_schema(&conn)?;
    tune_sqlite(&conn)?;

    let tx = conn.transaction()?;
    tx.execute(
        "INSERT OR REPLACE INTO releases (release_id, source_path, imported_at)
         VALUES (?1, ?2, datetime('now'))",
        params![release, vcf_path.display().to_string()],
    )?;
    tx.execute(
        "DELETE FROM release_records WHERE release_id = ?1",
        params![release],
    )?;

    let mut insert = tx.prepare(
        "INSERT INTO release_records (
            release_id, variant_key, row_number, chrom, pos, variation_id, allele_id,
            ref, alt, qual, filter, info, record_hash, raw_line
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
    )?;

    let mut count = 0_u64;
    for item in read_vcf_records(vcf_path)? {
        let (_, record) = item?;
        count += 1;
        insert_record(&mut insert, release, count, &record)?;
        if let Some(limit) = limit
            && count >= limit
        {
            break;
        }
        if count.is_multiple_of(250_000) {
            eprintln!("imported {count} records");
        }
    }

    drop(insert);
    tx.execute(
        "UPDATE releases SET record_count = ?2 WHERE release_id = ?1",
        params![release, count],
    )?;
    tx.commit()?;

    println!("imported {count} records into release {release}");
    Ok(())
}

fn diff_releases(
    old_release: &str,
    new_release: &str,
    old_path: &Path,
    new_path: &Path,
    out_dir: &Path,
    limit: Option<u64>,
) -> Result<()> {
    fs::create_dir_all(out_dir)
        .with_context(|| format!("creating output directory {}", out_dir.display()))?;
    let patch_path = out_dir.join("changes.jsonl.gz");
    let manifest_path = out_dir.join("manifest.json");

    let work_db_path = out_dir.join("diff-work.sqlite");
    if work_db_path.exists() {
        fs::remove_file(&work_db_path)
            .with_context(|| format!("removing old work database {}", work_db_path.display()))?;
    }
    let mut tmp = Connection::open(&work_db_path)
        .with_context(|| format!("opening work database {}", work_db_path.display()))?;
    tune_sqlite(&tmp)?;
    tmp.execute_batch(
        "
        CREATE TABLE old_records (
            variant_key TEXT PRIMARY KEY,
            record_hash TEXT NOT NULL,
            raw_line TEXT NOT NULL,
            seen INTEGER NOT NULL DEFAULT 0
        );
        ",
    )?;

    let tx = tmp.transaction()?;
    let mut insert_old = tx.prepare(
        "INSERT OR REPLACE INTO old_records (variant_key, record_hash, raw_line)
         VALUES (?1, ?2, ?3)",
    )?;

    let mut old_count = 0_u64;
    for item in read_vcf_records(old_path)? {
        let (_, record) = item?;
        old_count += 1;
        insert_old.execute(params![record.key, record.hash, record.raw_line])?;
        if let Some(limit) = limit
            && old_count >= limit
        {
            break;
        }
        if old_count.is_multiple_of(250_000) {
            eprintln!("indexed {old_count} old records");
        }
    }
    drop(insert_old);
    tx.commit()?;

    let patch_file = File::create(&patch_path)
        .with_context(|| format!("creating patch {}", patch_path.display()))?;
    let mut patch = BufWriter::new(GzEncoder::new(patch_file, Compression::best()));

    let mut added = 0_u64;
    let mut changed = 0_u64;
    let mut unchanged = 0_u64;
    let mut new_count = 0_u64;

    let tx = tmp.transaction()?;
    let mut select_old =
        tx.prepare("SELECT record_hash FROM old_records WHERE variant_key = ?1")?;
    let mut mark_seen = tx.prepare("UPDATE old_records SET seen = 1 WHERE variant_key = ?1")?;

    for item in read_vcf_records(new_path)? {
        let (_, record) = item?;
        new_count += 1;
        let old_hash: Option<String> = select_old
            .query_row(params![record.key], |row| row.get(0))
            .optional()?;

        match old_hash {
            None => {
                added += 1;
                write_patch_record(
                    &mut patch,
                    &PatchRecord::Added {
                        key: record.key.clone(),
                        new: PatchVcfRecord::from(&record),
                    },
                )?;
            }
            Some(old_hash) if old_hash == record.hash => {
                unchanged += 1;
                mark_seen.execute(params![record.key])?;
            }
            Some(old_hash) => {
                changed += 1;
                mark_seen.execute(params![record.key])?;
                write_patch_record(
                    &mut patch,
                    &PatchRecord::Changed {
                        key: record.key.clone(),
                        old_hash,
                        new: PatchVcfRecord::from(&record),
                    },
                )?;
            }
        }

        if let Some(limit) = limit
            && new_count >= limit
        {
            break;
        }
        if new_count.is_multiple_of(250_000) {
            eprintln!("compared {new_count} new records");
        }
    }

    drop(select_old);
    drop(mark_seen);
    tx.commit()?;

    let mut removed = 0_u64;
    let mut stmt = tmp.prepare(
        "SELECT variant_key, record_hash, raw_line FROM old_records WHERE seen = 0 ORDER BY variant_key",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        removed += 1;
        let record = PatchRecord::Removed {
            key: row.get(0)?,
            old_hash: row.get(1)?,
            old_raw_line: row.get(2)?,
        };
        write_patch_record(&mut patch, &record)?;
    }
    patch.flush()?;

    let manifest = PatchManifest {
        old_release: old_release.to_string(),
        new_release: new_release.to_string(),
        old_path: old_path.display().to_string(),
        new_path: new_path.display().to_string(),
        added,
        changed,
        removed,
        unchanged,
        old_records_scanned: old_count,
        new_records_scanned: new_count,
    };
    let manifest_file = File::create(&manifest_path)
        .with_context(|| format!("creating manifest {}", manifest_path.display()))?;
    serde_json::to_writer_pretty(BufWriter::new(manifest_file), &manifest)?;

    println!(
        "wrote patch: added={added} changed={changed} removed={removed} unchanged={unchanged}"
    );
    eprintln!("left diff work database at {}", work_db_path.display());
    Ok(())
}

fn apply_patch(
    db_path: &Path,
    old_release: &str,
    new_release: &str,
    patch_path: &Path,
) -> Result<()> {
    let mut conn = Connection::open(db_path)
        .with_context(|| format!("opening sqlite database {}", db_path.display()))?;
    setup_schema(&conn)?;
    tune_sqlite(&conn)?;

    let old_exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM releases WHERE release_id = ?1",
            params![old_release],
            |row| row.get(0),
        )
        .optional()?;
    if old_exists.is_none() {
        bail!(
            "old release {old_release} is not imported in {}",
            db_path.display()
        );
    }

    let tx = conn.transaction()?;
    tx.execute(
        "INSERT OR REPLACE INTO releases (release_id, source_path, imported_at)
         VALUES (?1, ?2, datetime('now'))",
        params![new_release, format!("patch:{}", patch_path.display())],
    )?;
    tx.execute(
        "DELETE FROM release_records WHERE release_id = ?1",
        params![new_release],
    )?;
    tx.execute(
        "INSERT INTO release_records (
            release_id, variant_key, row_number, chrom, pos, variation_id, allele_id,
            ref, alt, qual, filter, info, record_hash, raw_line
         )
         SELECT ?1, variant_key, row_number, chrom, pos, variation_id, allele_id,
            ref, alt, qual, filter, info, record_hash, raw_line
         FROM release_records
         WHERE release_id = ?2",
        params![new_release, old_release],
    )?;

    let mut delete_record =
        tx.prepare("DELETE FROM release_records WHERE release_id = ?1 AND variant_key = ?2")?;
    let mut insert = tx.prepare(
        "INSERT INTO release_records (
            release_id, variant_key, row_number, chrom, pos, variation_id, allele_id,
            ref, alt, qual, filter, info, record_hash, raw_line
         ) VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
    )?;

    let reader = open_text_reader(patch_path)?;
    let mut applied = 0_u64;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let record: PatchRecord = serde_json::from_str(&line)?;
        match record {
            PatchRecord::Added { key, new } => {
                insert_patch_record(&mut insert, new_release, &key, &new)?;
            }
            PatchRecord::Changed { key, new, .. } => {
                delete_record.execute(params![new_release, key])?;
                insert_patch_record(&mut insert, new_release, &key, &new)?;
            }
            PatchRecord::Removed { key, .. } => {
                delete_record.execute(params![new_release, key])?;
            }
        }
        applied += 1;
        if applied.is_multiple_of(250_000) {
            eprintln!("applied {applied} patch records");
        }
    }

    drop(delete_record);
    drop(insert);
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM release_records WHERE release_id = ?1",
        params![new_release],
        |row| row.get(0),
    )?;
    tx.execute(
        "UPDATE releases SET record_count = ?2 WHERE release_id = ?1",
        params![new_release, count],
    )?;
    tx.commit()?;

    println!("applied {applied} patch records into release {new_release}; records={count}");
    Ok(())
}

fn setup_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS releases (
            release_id TEXT PRIMARY KEY,
            source_path TEXT NOT NULL,
            imported_at TEXT NOT NULL,
            record_count INTEGER
        );

        CREATE TABLE IF NOT EXISTS release_records (
            release_id TEXT NOT NULL,
            variant_key TEXT NOT NULL,
            row_number INTEGER,
            chrom TEXT NOT NULL,
            pos INTEGER NOT NULL,
            variation_id TEXT NOT NULL,
            allele_id TEXT,
            ref TEXT NOT NULL,
            alt TEXT NOT NULL,
            qual TEXT NOT NULL,
            filter TEXT NOT NULL,
            info TEXT NOT NULL,
            record_hash TEXT NOT NULL,
            raw_line TEXT NOT NULL,
            PRIMARY KEY (release_id, variant_key)
        );

        CREATE INDEX IF NOT EXISTS idx_release_records_region
            ON release_records (release_id, chrom, pos);
        CREATE INDEX IF NOT EXISTS idx_release_records_variation
            ON release_records (variation_id);
        CREATE INDEX IF NOT EXISTS idx_release_records_allele
            ON release_records (allele_id);
        ",
    )?;
    Ok(())
}

fn tune_sqlite(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "cache_size", -200_000)?;
    Ok(())
}

fn read_vcf_records(path: &Path) -> Result<impl Iterator<Item = Result<(u64, VcfRecord)>>> {
    let reader = open_text_reader(path)?;
    let mut row_number = 0_u64;
    Ok(reader.lines().filter_map(move |line| match line {
        Ok(line) if line.starts_with('#') || line.trim().is_empty() => None,
        Ok(line) => {
            row_number += 1;
            Some(parse_vcf_record(row_number, line).map(|record| (row_number, record)))
        }
        Err(err) => Some(Err(err.into())),
    }))
}

fn open_text_reader(path: &Path) -> Result<Box<dyn BufRead>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let is_gz = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == "gz");
    if is_gz {
        Ok(Box::new(BufReader::new(MultiGzDecoder::new(file))))
    } else {
        Ok(Box::new(BufReader::new(file)))
    }
}

fn parse_vcf_record(row_number: u64, raw_line: String) -> Result<VcfRecord> {
    let columns: Vec<&str> = raw_line.split('\t').collect();
    if columns.len() < 8 {
        bail!(
            "row {row_number} has {} columns, expected at least 8",
            columns.len()
        );
    }

    let chrom = columns[0].to_string();
    let pos = columns[1]
        .parse::<i64>()
        .with_context(|| format!("invalid POS on row {row_number}: {}", columns[1]))?;
    let variation_id = columns[2].to_string();
    let reference = columns[3].to_string();
    let alternate = columns[4].to_string();
    let qual = columns[5].to_string();
    let filter = columns[6].to_string();
    let info = columns[7].to_string();
    let allele_id = info_value(&info, "ALLELEID").map(ToOwned::to_owned);
    let key = variant_key(
        &chrom,
        pos,
        &variation_id,
        &reference,
        &alternate,
        allele_id.as_deref(),
    );
    let hash = sha256_hex(raw_line.as_bytes());

    Ok(VcfRecord {
        chrom,
        pos,
        variation_id,
        reference,
        alternate,
        qual,
        filter,
        info,
        allele_id,
        key,
        hash,
        raw_line,
    })
}

fn variant_key(
    chrom: &str,
    pos: i64,
    variation_id: &str,
    reference: &str,
    alternate: &str,
    allele_id: Option<&str>,
) -> String {
    if !variation_id.is_empty() && variation_id != "." {
        format!("clinvar:{variation_id}")
    } else {
        format!(
            "loc:{chrom}:{pos}:{reference}:{alternate}:{}",
            allele_id.unwrap_or(".")
        )
    }
}

fn info_value<'a>(info: &'a str, key: &str) -> Option<&'a str> {
    info.split(';').find_map(|field| {
        let (name, value) = field.split_once('=')?;
        (name == key).then_some(value)
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn insert_record(
    stmt: &mut rusqlite::Statement<'_>,
    release: &str,
    row_number: u64,
    record: &VcfRecord,
) -> Result<()> {
    stmt.execute(params![
        release,
        record.key,
        row_number as i64,
        record.chrom,
        record.pos,
        record.variation_id,
        record.allele_id,
        record.reference,
        record.alternate,
        record.qual,
        record.filter,
        record.info,
        record.hash,
        record.raw_line,
    ])?;
    Ok(())
}

fn insert_patch_record(
    stmt: &mut rusqlite::Statement<'_>,
    release: &str,
    key: &str,
    record: &PatchVcfRecord,
) -> Result<()> {
    let parsed = parse_vcf_record(0, record.raw_line.clone())?;
    stmt.execute(params![
        release,
        key,
        parsed.chrom,
        parsed.pos,
        parsed.variation_id,
        parsed.allele_id,
        parsed.reference,
        parsed.alternate,
        parsed.qual,
        parsed.filter,
        parsed.info,
        record.hash,
        record.raw_line,
    ])?;
    Ok(())
}

fn write_patch_record(writer: &mut impl Write, record: &PatchRecord) -> Result<()> {
    serde_json::to_writer(&mut *writer, record)?;
    writer.write_all(b"\n")?;
    Ok(())
}

fn write_plain_vcf(input: &Path, output: &Path, limit: Option<u64>) -> Result<()> {
    let mut reader = open_text_reader(input)?;
    let output_file =
        File::create(output).with_context(|| format!("creating {}", output.display()))?;
    let mut writer = BufWriter::new(output_file);
    let mut data_rows = 0_u64;
    let mut line = String::new();

    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }

        let is_header = line.starts_with('#');
        if !is_header {
            data_rows += 1;
        }
        writer.write_all(line.as_bytes())?;

        if !is_header
            && let Some(limit) = limit
            && data_rows >= limit
        {
            break;
        }
    }

    writer.flush()?;
    println!(
        "wrote {data_rows} data rows to plain VCF {}",
        output.display()
    );
    Ok(())
}

fn index_rows(file_path: &Path, out_path: &Path, stride: u64) -> Result<()> {
    if stride == 0 {
        bail!("--stride must be greater than zero");
    }
    if file_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == "gz")
    {
        bail!("row byte indexes require a plain uncompressed VCF; use write-plain first");
    }

    let file = File::open(file_path).with_context(|| format!("opening {}", file_path.display()))?;
    let file_size = file.metadata()?.len();
    let mut reader = BufReader::new(file);
    let mut checkpoints = Vec::new();
    let mut row = 0_u64;
    let mut offset = 0_u64;
    let mut header_bytes = 0_u64;
    let mut hasher = Sha256::new();
    let mut line = String::new();

    loop {
        line.clear();
        let start_offset = offset;
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }
        offset += bytes as u64;
        hasher.update(line.as_bytes());

        if line.starts_with('#') {
            header_bytes = offset;
            continue;
        }

        row += 1;
        if row == 1 || (row - 1).is_multiple_of(stride) {
            let checkpoint = checkpoint_from_line(row, start_offset, &line)?;
            checkpoints.push(checkpoint);
        }

        if row.is_multiple_of(250_000) {
            eprintln!("indexed {row} rows");
        }
    }

    let data_file = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data.vcf")
        .to_string();
    let index = RowIndex {
        format: "clinvar-row-byte-index".to_string(),
        version: 1,
        data_file,
        file_size,
        file_hash_sha256: hex::encode(hasher.finalize()),
        row_count: row,
        header_bytes,
        stride,
        checkpoints,
    };

    let out = File::create(out_path).with_context(|| format!("creating {}", out_path.display()))?;
    serde_json::to_writer_pretty(BufWriter::new(out), &index)?;
    println!(
        "wrote row index {} for {row} rows with stride {stride}",
        out_path.display()
    );
    Ok(())
}

fn checkpoint_from_line(row: u64, offset: u64, line: &str) -> Result<RowCheckpoint> {
    let mut columns = line.split('\t');
    let chrom = columns.next().unwrap_or("").to_string();
    let pos = columns.next().and_then(|value| value.parse::<u64>().ok());
    let id = columns.next().unwrap_or(".").to_string();
    Ok(RowCheckpoint {
        row,
        offset,
        chrom,
        pos,
        id,
    })
}

fn index_ids(file_path: &Path, out_path: &Path) -> Result<()> {
    if file_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == "gz")
    {
        bail!("id byte indexes require a plain uncompressed VCF; use write-plain first");
    }

    let file = File::open(file_path).with_context(|| format!("opening {}", file_path.display()))?;
    let file_size = file.metadata()?.len();
    let mut reader = BufReader::new(file);
    let mut records = Vec::new();
    let mut row = 0_u64;
    let mut offset = 0_u64;
    let mut hasher = Sha256::new();
    let mut line = String::new();

    loop {
        line.clear();
        let start_offset = offset;
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }
        offset += bytes as u64;
        hasher.update(line.as_bytes());

        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }

        row += 1;
        let mut columns = line.split('\t');
        let chrom = columns.next().unwrap_or("").to_string();
        let pos = columns.next().and_then(|value| value.parse::<u64>().ok());
        let id = columns.next().unwrap_or(".").to_string();
        if id != "." && !id.is_empty() {
            records.push(IdIndexRecord {
                id,
                row,
                offset: start_offset,
                length: bytes as u64,
                chrom,
                pos,
            });
        }

        if row.is_multiple_of(250_000) {
            eprintln!("indexed {row} ids");
        }
    }

    records.sort_by(|a, b| {
        a.id.parse::<u64>()
            .ok()
            .cmp(&b.id.parse::<u64>().ok())
            .then_with(|| a.id.cmp(&b.id))
    });

    let data_file = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data.vcf")
        .to_string();
    let index = IdIndex {
        format: "clinvar-id-byte-index".to_string(),
        version: 1,
        data_file,
        file_size,
        file_hash_sha256: hex::encode(hasher.finalize()),
        row_count: row,
        records,
    };

    let out = File::create(out_path).with_context(|| format!("creating {}", out_path.display()))?;
    serde_json::to_writer(BufWriter::new(out), &index)?;
    println!("wrote id index {} for {row} rows", out_path.display());
    Ok(())
}

fn index_positions(file_path: &Path, out_path: &Path) -> Result<()> {
    if file_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == "gz")
    {
        bail!("position byte indexes require a plain uncompressed VCF; use write-plain first");
    }

    let file = File::open(file_path).with_context(|| format!("opening {}", file_path.display()))?;
    let file_size = file.metadata()?.len();
    let mut reader = BufReader::new(file);
    let mut row = 0_u64;
    let mut offset = 0_u64;
    let mut hasher = Sha256::new();
    let mut line = String::new();
    let mut entries: Vec<PositionIndexEntry> = Vec::new();
    let mut current_chrom = String::new();
    let mut current_pos = 0_u64;
    let mut current_records: Vec<PositionIndexRecord> = Vec::new();

    loop {
        line.clear();
        let start_offset = offset;
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }
        offset += bytes as u64;
        hasher.update(line.as_bytes());

        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }

        row += 1;
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() < 5 {
            bail!(
                "row {row} has {} columns, expected at least 5",
                columns.len()
            );
        }
        let chrom = columns[0].to_string();
        let pos = columns[1]
            .parse::<u64>()
            .with_context(|| format!("invalid POS on row {row}: {}", columns[1]))?;
        let record = PositionIndexRecord {
            id: columns[2].to_string(),
            row,
            offset: start_offset,
            length: bytes as u64,
            reference: columns[3].to_string(),
            alternate: columns[4].to_string(),
        };

        if current_records.is_empty() {
            current_chrom = chrom;
            current_pos = pos;
            current_records.push(record);
        } else if current_chrom == chrom && current_pos == pos {
            current_records.push(record);
        } else {
            entries.push(PositionIndexEntry {
                chrom: std::mem::take(&mut current_chrom),
                pos: current_pos,
                records: std::mem::take(&mut current_records),
            });
            current_chrom = chrom;
            current_pos = pos;
            current_records.push(record);
        }

        if row.is_multiple_of(250_000) {
            eprintln!("indexed {row} positions");
        }
    }

    if !current_records.is_empty() {
        entries.push(PositionIndexEntry {
            chrom: current_chrom,
            pos: current_pos,
            records: current_records,
        });
    }

    let data_file = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data.vcf")
        .to_string();
    let index = PositionIndex {
        format: "clinvar-position-byte-index".to_string(),
        version: 1,
        data_file,
        file_size,
        file_hash_sha256: hex::encode(hasher.finalize()),
        row_count: row,
        positions: entries,
    };

    let out = File::create(out_path).with_context(|| format!("creating {}", out_path.display()))?;
    serde_json::to_writer(BufWriter::new(out), &index)?;
    println!("wrote position index {} for {row} rows", out_path.display());
    Ok(())
}

fn rows_local(file_path: &Path, index_path: &Path, start: u64, count: u64) -> Result<()> {
    if start == 0 {
        bail!("--start is 1-based and must be greater than zero");
    }
    if count == 0 {
        return Ok(());
    }
    let index_file =
        File::open(index_path).with_context(|| format!("opening {}", index_path.display()))?;
    let index: RowIndex = serde_json::from_reader(BufReader::new(index_file))?;
    let end = start
        .checked_add(count - 1)
        .context("row range overflows u64")?;
    let (range_start, range_end, checkpoint_row) = indexed_byte_range(&index, start, end)?;

    eprintln!(
        "HTTP Range equivalent: bytes={range_start}-{}",
        range_end
            .map(|offset| (offset - 1).to_string())
            .unwrap_or_default()
    );

    let mut file =
        File::open(file_path).with_context(|| format!("opening {}", file_path.display()))?;
    file.seek(SeekFrom::Start(range_start))?;
    let take_len = range_end.unwrap_or(index.file_size) - range_start;
    let mut reader = BufReader::new(file.take(take_len));
    let mut current_row = checkpoint_row;
    let mut line = String::new();

    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }
        if current_row >= start && current_row <= end {
            print!("{line}");
        }
        if current_row >= end {
            break;
        }
        current_row += 1;
    }

    Ok(())
}

fn indexed_byte_range(index: &RowIndex, start: u64, end: u64) -> Result<(u64, Option<u64>, u64)> {
    if start > end {
        bail!("start row must be <= end row");
    }
    if end > index.row_count {
        bail!(
            "requested row {end}, but index only has {} rows",
            index.row_count
        );
    }

    let start_checkpoint = index
        .checkpoints
        .iter()
        .rev()
        .find(|checkpoint| checkpoint.row <= start)
        .context("index has no checkpoint before requested start row")?;
    let end_exclusive_row = end + 1;
    let end_offset = index
        .checkpoints
        .iter()
        .find(|checkpoint| checkpoint.row >= end_exclusive_row)
        .map(|checkpoint| checkpoint.offset);

    Ok((start_checkpoint.offset, end_offset, start_checkpoint.row))
}

fn build_chunks(input: &Path, out_dir: &Path, options: ChunkBuildOptions<'_>) -> Result<()> {
    if options.bin_size == 0 {
        bail!("--bin-size must be greater than zero");
    }
    if options.row_stride == 0 {
        bail!("--row-stride must be greater than zero");
    }

    fs::create_dir_all(out_dir)
        .with_context(|| format!("creating output directory {}", out_dir.display()))?;
    let chunks_dir = out_dir.join("chunks");
    fs::create_dir_all(&chunks_dir)
        .with_context(|| format!("creating chunks directory {}", chunks_dir.display()))?;

    let region = options.region.map(parse_region).transpose()?;
    let mut reader = open_text_reader(input)?;
    let mut header = Vec::new();
    let mut line = String::new();
    let mut active: Option<ActiveChunk> = None;
    let mut chunks = Vec::new();
    let mut total_rows = 0_u64;
    let mut current_key: Option<(String, u64, u64)> = None;
    let mut current_part = 1_u32;

    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }

        if line.starts_with('#') {
            header.extend_from_slice(line.as_bytes());
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }

        let (chrom, pos) = chrom_pos_from_line(total_rows + 1, &line)?;
        if let Some(region) = &region
            && (chrom != region.chrom || pos < region.start || pos > region.end)
        {
            continue;
        }
        let (start, end) = coordinate_bin(pos, options.bin_size)?;
        let key = (chrom.clone(), start, end);

        if current_key.as_ref() != Some(&key) {
            if let Some(chunk) = active.take() {
                chunks.push(finalize_chunk(chunk, out_dir, options.row_stride)?);
            }
            current_key = Some(key);
            current_part = 1;
            active = Some(open_chunk(
                &chunks_dir,
                &header,
                &chrom,
                start,
                end,
                current_part,
            )?);
        }

        let projected_bytes = active
            .as_ref()
            .map(|chunk| chunk.estimated_bytes + bytes as u64)
            .unwrap_or(bytes as u64);
        if projected_bytes > options.max_bytes
            && active.as_ref().is_some_and(|chunk| chunk.row_count > 0)
        {
            let chunk = active.take().context("missing active chunk")?;
            chunks.push(finalize_chunk(chunk, out_dir, options.row_stride)?);
            current_part += 1;
            active = Some(open_chunk(
                &chunks_dir,
                &header,
                &chrom,
                start,
                end,
                current_part,
            )?);
        }

        let chunk = active.as_mut().context("missing active chunk")?;
        chunk.writer.write_all(line.as_bytes())?;
        chunk.row_count += 1;
        chunk.estimated_bytes += bytes as u64;
        total_rows += 1;

        if let Some(limit) = options.limit
            && total_rows >= limit
        {
            break;
        }
        if total_rows.is_multiple_of(250_000) {
            eprintln!("chunked {total_rows} rows");
        }
    }

    if let Some(chunk) = active.take() {
        chunks.push(finalize_chunk(chunk, out_dir, options.row_stride)?);
    }

    let manifest = ChunkManifest {
        format: "clinpatch-coordinate-chunks".to_string(),
        version: 1,
        assembly: options.assembly.to_string(),
        source_file: input.display().to_string(),
        bin_size: options.bin_size,
        max_bytes: options.max_bytes,
        row_stride: options.row_stride,
        chunk_count: chunks.len() as u64,
        row_count: total_rows,
        chunks,
    };
    let manifest_path = out_dir.join("manifest.json");
    let manifest_file = File::create(&manifest_path)
        .with_context(|| format!("creating {}", manifest_path.display()))?;
    serde_json::to_writer_pretty(BufWriter::new(manifest_file), &manifest)?;

    println!(
        "wrote {} chunks with {total_rows} rows to {}",
        manifest.chunk_count,
        out_dir.display()
    );
    Ok(())
}

fn chrom_pos_from_line(row_number: u64, line: &str) -> Result<(String, u64)> {
    let mut columns = line.split('\t');
    let chrom = columns
        .next()
        .filter(|value| !value.is_empty())
        .with_context(|| format!("row {row_number} is missing CHROM"))?
        .to_string();
    let pos = columns
        .next()
        .with_context(|| format!("row {row_number} is missing POS"))?
        .parse::<u64>()
        .with_context(|| format!("invalid POS on row {row_number}"))?;
    Ok((chrom, pos))
}

fn coordinate_bin(pos: u64, bin_size: u64) -> Result<(u64, u64)> {
    if pos == 0 {
        bail!("VCF POS must be 1-based");
    }
    let start = ((pos - 1) / bin_size) * bin_size + 1;
    let end = start
        .checked_add(bin_size - 1)
        .context("coordinate bin overflows u64")?;
    Ok((start, end))
}

fn parse_region(region: &str) -> Result<RegionFilter> {
    let (chrom, range) = region
        .split_once(':')
        .with_context(|| format!("region must be CHROM:START-END, got {region}"))?;
    let (start, end) = range
        .split_once('-')
        .with_context(|| format!("region must be CHROM:START-END, got {region}"))?;
    let start = start
        .replace(',', "")
        .parse::<u64>()
        .with_context(|| format!("invalid region start in {region}"))?;
    let end = end
        .replace(',', "")
        .parse::<u64>()
        .with_context(|| format!("invalid region end in {region}"))?;
    if start == 0 || start > end {
        bail!("invalid region coordinates in {region}");
    }
    Ok(RegionFilter {
        chrom: normalize_query_chrom(chrom),
        start,
        end,
    })
}

fn normalize_query_chrom(raw: &str) -> String {
    let chrom = raw.strip_prefix("chr").unwrap_or(raw);
    if chrom == "M" {
        "MT".to_string()
    } else {
        chrom.to_string()
    }
}

fn open_chunk(
    chunks_dir: &Path,
    header: &[u8],
    chrom: &str,
    start: u64,
    end: u64,
    part: u32,
) -> Result<ActiveChunk> {
    let chrom_dir = chunks_dir.join(safe_path_segment(chrom));
    fs::create_dir_all(&chrom_dir)
        .with_context(|| format!("creating chunk directory {}", chrom_dir.display()))?;
    let filename = if part == 1 {
        format!("{start:012}-{end:012}.vcf")
    } else {
        format!("{start:012}-{end:012}.part{part}.vcf")
    };
    let data_path = chrom_dir.join(filename);
    let file =
        File::create(&data_path).with_context(|| format!("creating {}", data_path.display()))?;
    let mut writer = BufWriter::new(file);
    writer.write_all(header)?;

    Ok(ActiveChunk {
        chrom: chrom.to_string(),
        start,
        end,
        part,
        data_path,
        writer,
        row_count: 0,
        estimated_bytes: header.len() as u64,
    })
}

fn finalize_chunk(
    mut chunk: ActiveChunk,
    out_dir: &Path,
    row_stride: u64,
) -> Result<ChunkManifestEntry> {
    chunk.writer.flush()?;
    drop(chunk.writer);

    let rows_index = chunk.data_path.with_extension("vcf.rows.json");
    let ids_index = chunk.data_path.with_extension("vcf.ids.json");
    let positions_index = chunk.data_path.with_extension("vcf.positions.json");

    index_rows(&chunk.data_path, &rows_index, row_stride)?;
    index_ids(&chunk.data_path, &ids_index)?;
    index_positions(&chunk.data_path, &positions_index)?;

    let index_file =
        File::open(&rows_index).with_context(|| format!("opening {}", rows_index.display()))?;
    let row_index: RowIndex = serde_json::from_reader(BufReader::new(index_file))?;

    Ok(ChunkManifestEntry {
        chrom: chunk.chrom,
        start: chunk.start,
        end: chunk.end,
        part: chunk.part,
        data_file: manifest_path(out_dir, &chunk.data_path)?,
        rows_index: manifest_path(out_dir, &rows_index)?,
        ids_index: manifest_path(out_dir, &ids_index)?,
        positions_index: manifest_path(out_dir, &positions_index)?,
        file_size: row_index.file_size,
        row_count: row_index.row_count,
        file_hash_sha256: row_index.file_hash_sha256,
    })
}

fn manifest_path(root: &Path, path: &Path) -> Result<String> {
    let relative = path.strip_prefix(root).with_context(|| {
        format!(
            "{} is not inside manifest root {}",
            path.display(),
            root.display()
        )
    })?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn safe_path_segment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn build_gene_index(gtf: &Path, out_path: &Path, assembly: &str, source_url: &str) -> Result<()> {
    if let Some(parent) = out_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating output directory {}", parent.display()))?;
    }

    let reader = open_text_reader(gtf)?;
    let mut genes = Vec::new();
    let mut skipped = 0_u64;

    for line in reader.lines() {
        let line = line?;
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }

        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() < 9 || columns[2] != "gene" {
            continue;
        }

        let Some(chrom) = normalize_gene_chrom(columns[0]) else {
            skipped += 1;
            continue;
        };
        let start = columns[3]
            .parse::<u64>()
            .with_context(|| format!("invalid gene start: {}", columns[3]))?;
        let end = columns[4]
            .parse::<u64>()
            .with_context(|| format!("invalid gene end: {}", columns[4]))?;
        let attrs = parse_gtf_attrs(columns[8]);
        let Some(symbol) = attrs.get("gene_name") else {
            skipped += 1;
            continue;
        };
        let Some(gene_id) = attrs.get("gene_id") else {
            skipped += 1;
            continue;
        };
        let Some(gene_type) = attrs.get("gene_type") else {
            skipped += 1;
            continue;
        };

        genes.push(GeneIndexEntry {
            symbol: symbol.clone(),
            symbol_norm: symbol.to_ascii_uppercase(),
            gene_id: gene_id.clone(),
            gene_type: gene_type.clone(),
            chrom,
            start,
            end,
            strand: columns[6].to_string(),
        });
    }

    genes.sort_by(|a, b| {
        a.symbol_norm
            .cmp(&b.symbol_norm)
            .then_with(|| a.chrom.cmp(&b.chrom))
            .then_with(|| a.start.cmp(&b.start))
            .then_with(|| a.end.cmp(&b.end))
    });

    let index = GeneIndex {
        format: "clinpatch-gene-coordinate-index".to_string(),
        version: 1,
        assembly: assembly.to_string(),
        source_url: source_url.to_string(),
        gene_count: genes.len() as u64,
        genes,
    };
    let out = File::create(out_path).with_context(|| format!("creating {}", out_path.display()))?;
    serde_json::to_writer(BufWriter::new(out), &index)?;

    println!(
        "wrote {} genes to {} ({skipped} skipped)",
        index.gene_count,
        out_path.display()
    );
    Ok(())
}

fn normalize_gene_chrom(raw: &str) -> Option<String> {
    let chrom = raw.strip_prefix("chr").unwrap_or(raw);
    let chrom = if chrom == "M" { "MT" } else { chrom };
    if matches!(chrom, "X" | "Y" | "MT") {
        return Some(chrom.to_string());
    }
    let numeric = chrom.parse::<u8>().ok()?;
    (1..=22).contains(&numeric).then(|| numeric.to_string())
}

fn parse_gtf_attrs(raw: &str) -> std::collections::HashMap<String, String> {
    let mut attrs = std::collections::HashMap::new();
    for field in raw.split(';') {
        let field = field.trim();
        if field.is_empty() {
            continue;
        }
        let Some((key, value)) = field.split_once(' ') else {
            continue;
        };
        attrs.insert(key.to_string(), value.trim_matches('"').to_string());
    }
    attrs
}

fn serve_static(root: &Path, bind: &str) -> Result<()> {
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalizing {}", root.display()))?;
    let server = Server::http(bind).map_err(|err| anyhow::anyhow!(err.to_string()))?;
    println!("serving {} at http://{bind}", root.display());

    for request in server.incoming_requests() {
        if let Err(err) = handle_static_request(&root, request) {
            eprintln!("request error: {err:#}");
        }
    }
    Ok(())
}

fn handle_static_request(root: &Path, request: tiny_http::Request) -> Result<()> {
    let method = request.method().clone();
    if method != Method::Get && method != Method::Head {
        let response =
            Response::from_string("method not allowed").with_status_code(StatusCode(405));
        request.respond(response)?;
        return Ok(());
    }

    let url_path = request.url().split('?').next().unwrap_or("/");
    let relative = url_path.trim_start_matches('/');
    if relative.contains("..") {
        let response = Response::from_string("not found").with_status_code(StatusCode(404));
        request.respond(response)?;
        return Ok(());
    }

    let path = if relative.is_empty() {
        root.join("index.html")
    } else {
        root.join(relative)
    };
    let path = match path.canonicalize() {
        Ok(path) if path.starts_with(root) && path.is_file() => path,
        _ => {
            let response = Response::from_string("not found").with_status_code(StatusCode(404));
            request.respond(response)?;
            return Ok(());
        }
    };

    let file_size = path.metadata()?.len();
    let range = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Range"))
        .and_then(|header| parse_range_header(header.value.as_str(), file_size));

    let mut headers = vec![
        Header::from_bytes(&b"Accept-Ranges"[..], &b"bytes"[..]).unwrap(),
        Header::from_bytes(&b"Content-Type"[..], content_type(&path).as_bytes()).unwrap(),
    ];

    let mut file = File::open(&path)?;
    let response = if let Some((start, end)) = range {
        let len = end - start + 1;
        file.seek(SeekFrom::Start(start))?;
        headers.push(
            Header::from_bytes(
                &b"Content-Range"[..],
                format!("bytes {start}-{end}/{file_size}").as_bytes(),
            )
            .unwrap(),
        );
        headers
            .push(Header::from_bytes(&b"Content-Length"[..], len.to_string().as_bytes()).unwrap());
        let body: Box<dyn Read + Send> = if method == Method::Head {
            Box::new(empty())
        } else {
            Box::new(file.take(len))
        };
        Response::new(StatusCode(206), headers, body, Some(len as usize), None)
    } else {
        headers.push(
            Header::from_bytes(&b"Content-Length"[..], file_size.to_string().as_bytes()).unwrap(),
        );
        let body: Box<dyn Read + Send> = if method == Method::Head {
            Box::new(empty())
        } else {
            Box::new(file)
        };
        Response::new(
            StatusCode(200),
            headers,
            body,
            Some(file_size as usize),
            None,
        )
    };

    request.respond(response)?;
    Ok(())
}

fn parse_range_header(header: &str, file_size: u64) -> Option<(u64, u64)> {
    let range = header.strip_prefix("bytes=")?;
    let (start, end) = range.split_once('-')?;
    let start = start.parse::<u64>().ok()?;
    let end = if end.is_empty() {
        file_size.checked_sub(1)?
    } else {
        end.parse::<u64>().ok()?
    };
    (start <= end && end < file_size).then_some((start, end))
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => "application/json",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("html") => "text/html",
        Some("gz") => "application/gzip",
        Some("vcf") => "text/plain",
        Some("txt") => "text/plain",
        _ => "application/octet-stream",
    }
}
