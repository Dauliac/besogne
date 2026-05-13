use crate::ir::BesogneIR;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const MAGIC: &[u8; 8] = b"BESOGNE\0";

/// Embed IR into an output binary by copying self and appending IR
pub fn emit(output_path: &Path, ir: &BesogneIR) -> Result<(), crate::error::BesogneError> {
    let self_path = std::env::current_exe()
        .map_err(|e| crate::error::BesogneError::Embed(format!("cannot find own binary: {e}")))?;

    // Copy the compiler binary as the base
    fs::copy(&self_path, output_path)
        .map_err(|e| crate::error::BesogneError::Embed(format!("cannot copy binary to {}: {e}", output_path.display())))?;

    // Make it executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(output_path, perms)
            .map_err(|e| crate::error::BesogneError::Embed(format!("cannot set permissions: {e}")))?;
    }

    // Serialize IR as JSON (bincode can't handle serde_json::Value)
    let ir_bytes = serde_json::to_vec(ir)
        .map_err(|e| crate::error::BesogneError::Embed(format!("cannot serialize IR: {e}")))?;

    // Append: [IR bytes] [IR length as u64 LE] [MAGIC]
    let mut file = OpenOptions::new()
        .append(true)
        .open(output_path)
        .map_err(|e| crate::error::BesogneError::Embed(format!("cannot open output for append: {e}")))?;

    file.write_all(&ir_bytes)
        .map_err(|e| crate::error::BesogneError::Embed(format!("cannot write IR: {e}")))?;
    file.write_all(&(ir_bytes.len() as u64).to_le_bytes())
        .map_err(|e| crate::error::BesogneError::Embed(format!("cannot write IR length: {e}")))?;
    file.write_all(MAGIC)
        .map_err(|e| crate::error::BesogneError::Embed(format!("cannot write magic: {e}")))?;

    Ok(())
}

/// Try to extract IR from the current binary (trailer protocol)
pub fn extract_ir_from_self() -> Option<BesogneIR> {
    let self_path = std::env::current_exe().ok()?;
    extract_ir_from_binary(&self_path)
}

fn extract_ir_from_binary(path: &Path) -> Option<BesogneIR> {
    let mut file = File::open(path).ok()?;
    let file_len = file.metadata().ok()?.len();

    // Need at least 16 bytes for footer (8 bytes length + 8 bytes magic)
    if file_len < 16 {
        return None;
    }

    // Read footer: [IR length: u64 LE] [MAGIC: 8 bytes]
    file.seek(SeekFrom::End(-16)).ok()?;
    let mut footer = [0u8; 16];
    file.read_exact(&mut footer).ok()?;

    // Check magic
    if &footer[8..16] != MAGIC {
        return None;
    }

    // Read IR length
    let ir_len = u64::from_le_bytes(footer[0..8].try_into().ok()?);

    if ir_len == 0 || ir_len > file_len - 16 {
        return None;
    }

    // Read IR bytes
    file.seek(SeekFrom::End(-16 - ir_len as i64)).ok()?;
    let mut ir_bytes = vec![0u8; ir_len as usize];
    file.read_exact(&mut ir_bytes).ok()?;

    // Deserialize
    match serde_json::from_slice(&ir_bytes) {
        Ok(ir) => Some(ir),
        Err(e) => {
            eprintln!("besogne: IR extraction failed: {e} (ir_len={ir_len})");
            None
        }
    }
}
