use std::env;
use std::fs;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use crate::*;

const DISK_BYTES: u64 = 64 * 1024 * 1024;
const SECTOR_BYTES: u64 = 512;

fn read_at(file: &mut File, offset: u64, buf: &mut [u8]) -> Result<(), String> {
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    file.read_exact(buf).map_err(|e| e.to_string())
}

fn verify_disk_layout(disk_img: &Path) -> Result<(), String> {
    let mut file = File::open(disk_img).map_err(|e| e.to_string())?;

    let mut header = [0u8; 512];
    read_at(&mut file, SECTOR_BYTES, &mut header)?;

    if &header[..8] != b"EFI PART" {
        return Err(format!("{} has no primary GPT header", disk_img.display()));
    }

    let current_lba = u64::from_le_bytes(header[24..32].try_into().unwrap());
    let entry_lba = u64::from_le_bytes(header[72..80].try_into().unwrap());
    let entry_count = u32::from_le_bytes(header[80..84].try_into().unwrap());
    let entry_size = u32::from_le_bytes(header[84..88].try_into().unwrap());

    if current_lba != 1 || entry_lba != 2 || entry_count != 128 || entry_size != 128 {
        return Err(format!(
            "{} has a GPT layout xtask does not recognize",
            disk_img.display()
        ));
    }

    let mut entry = [0u8; 128];
    read_at(&mut file, entry_lba * SECTOR_BYTES, &mut entry)?;

    let start = u64::from_le_bytes(entry[32..40].try_into().unwrap());
    let end = u64::from_le_bytes(entry[40..48].try_into().unwrap());
    let expected_end = PART_START + PART_SECTORS - 1;

    if start != PART_START || end != expected_end {
        return Err(format!(
            "{} partition 1 is {}..{}, expected {}..{}",
            disk_img.display(),
            start,
            end,
            PART_START,
            expected_end
        ));
    }

    let mut verify = Command::new("sgdisk");
    verify.arg("-v").arg(disk_img);
    run_cmd(verify, "verifying disk GPT")
}

pub fn ensure_disk_layout(disk_img: &Path) -> Result<(), String> {
    if !disk_img.exists() {
        let file = File::create(disk_img).map_err(|e| e.to_string())?;
        file.set_len(DISK_BYTES).map_err(|e| e.to_string())?;
    }

    let mut file = File::open(disk_img).map_err(|e| e.to_string())?;
    let actual_size = file.metadata().map_err(|e| e.to_string())?.len();

    if actual_size != DISK_BYTES {
        return Err(format!(
            "{} is {} bytes; expected {}",
            disk_img.display(),
            actual_size,
            DISK_BYTES
        ));
    }

    let mut primary_metadata = [0u8; 34 * 512];
    let mut backup_metadata = [0u8; 33 * 512];

    read_at(&mut file, 0, &mut primary_metadata)?;
    read_at(
        &mut file,
        DISK_BYTES - backup_metadata.len() as u64,
        &mut backup_metadata,
    )?;

    let has_primary_gpt = &primary_metadata[512..520] == b"EFI PART";

    if !has_primary_gpt {
        // A blank GPT area can be rebuilt without touching the ext2 partition,
        // which starts at PART_START. Anything else may be a damaged or
        // different partition table and needs investigation.
        if primary_metadata.iter().any(|&byte| byte != 0)
            || backup_metadata.iter().any(|&byte| byte != 0)
        {
            return Err(format!(
                "{} has missing or damaged GPT metadata; refusing automatic repair",
                disk_img.display()
            ));
        }

        let end = PART_START + PART_SECTORS - 1;
        let mut partition = Command::new("sgdisk");
        partition
            .arg("-n")
            .arg(format!("1:{}:{}", PART_START, end))
            .args(["-t", "1:8300"])
            .arg(disk_img);
        run_cmd(partition, "creating disk GPT")?;
    }

    verify_disk_layout(disk_img)
}
