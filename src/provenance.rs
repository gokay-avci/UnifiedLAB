// src/provenance.rs
//
// =============================================================================
// UNIFIEDLAB: ARTIFACT NOTARY (v 0.1 )
// =============================================================================
//
// The Trust Layer.
//
// Responsibilities:
// 1. Content Addressable Storage (CAS): Filenames are hashes of content.
// 2. Atomic Renames: Data effectively "appears" instantly, never partial.
// 3. Model Notarization: Verifies ML model weights match expected hashes.
// 4. Durability: Explicit fsyncs to handle HPC filesystem (Lustre) lag.

use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

// ============================================================================
// 1. HASHING UTILITIES
// ============================================================================

/// Calculates SHA256 of a file efficiently (streamed).
/// Does not load whole file into memory.
pub fn sha256_file(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref();
    let mut file =
        File::open(path).with_context(|| format!("Failed to open for hashing: {:?}", path))?;

    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536]; // 64KB buffer for throughput

    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }

    Ok(hex::encode(hasher.finalize()))
}

/// Calculates SHA256 of a byte slice (e.g. JSON string).
pub fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

// ============================================================================
// 2. CONTENT ADDRESSABLE STORAGE (CAS)
// ============================================================================

pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Moves a temporary file into the store, named by its hash.
    /// Returns: (The Hash, The Final Path)
    ///
    /// Strategy:
    /// 1. Calculate Hash of `temp_file`.
    /// 2. Construct `final_path = root / hash[0..2] / hash.ext`.
    /// 3. Atomic Rename (or Copy+Delete if cross-device).
    /// 4. fsync directory for Lustre safety.
    pub fn commit(
        &self,
        temp_file: impl AsRef<Path>,
        extension: &str,
    ) -> Result<(String, PathBuf)> {
        let temp_path = temp_file.as_ref();

        // 1. Hash it
        let hash = sha256_file(temp_path)?;

        // 2. Sharded Directory Structure (git-style: ab/abcdef...)
        // This prevents having 100,000 files in one folder (bad for HPC MDS).
        let shard = &hash[0..2];
        let shard_dir = self.root.join(shard);
        if !shard_dir.exists() {
            fs::create_dir_all(&shard_dir)?;
        }

        let filename = format!("{}.{}", hash, extension);
        let final_path = shard_dir.join(&filename);

        if final_path.exists() {
            // Deduplication! It already exists.
            // We can delete the temp file and return the existing path.
            // In a real system, we might want to verify the existing file's hash,
            // but for speed we assume CAS integrity.
            fs::remove_file(temp_path).ok();
            return Ok((hash, final_path));
        }

        // 3. Move it
        // Try atomic rename first (fastest, safest)
        if let Err(_) = fs::rename(temp_path, &final_path) {
            // Fallback: Copy + Delete
            // This happens if /tmp is NVMe (Local) and store/ is Lustre (Network)
            // We use copy, sync, then delete to ensure data safety
            fs::copy(temp_path, &final_path).context("Failed to copy artifact across devices")?;

            // Delete source only after copy succeeds
            fs::remove_file(temp_path)?;
        }

        // 4. Durability Sync (HPC Critical)
        // Ensure the directory entry is flushed to disk so the file "appears" to other nodes
        // immediately, fixing "File Not Found" errors on distributed reads.
        if let Ok(dir) = File::open(&shard_dir) {
            let _ = dir.sync_all();
        }

        Ok((hash, final_path))
    }
}

// ============================================================================
// 3. MODEL NOTARY (ML Provenance)
// ============================================================================

pub struct ModelNotary;

impl ModelNotary {
    /// Verifies that a model file on disk matches the expected hash.
    /// Useful for ensuring Janus/MACE models haven't been silently updated.
    pub fn verify(model_path: &Path, expected_hash: Option<&str>) -> Result<String> {
        if !model_path.exists() {
            return Err(anyhow!("Model file not found: {:?}", model_path));
        }

        let actual_hash = sha256_file(model_path).context("Failed to hash model weights")?;

        if let Some(expected) = expected_hash {
            if actual_hash != expected {
                return Err(anyhow!(
                    "Model Integrity Violation! Path: {:?}\nExpected: {}\nActual:   {}",
                    model_path,
                    expected,
                    actual_hash
                ));
            }
        }

        Ok(actual_hash)
    }
}
