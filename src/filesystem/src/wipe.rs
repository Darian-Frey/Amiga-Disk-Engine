//! Wiping what a volume no longer uses: the slack after each file's last
//! byte, and the blocks the bitmap marks free (F-031).
//!
//! Both destroy what a forensic reading would want — a deleted file's data,
//! an older version's tail — which is why they are separate, explicit edits
//! saved only to a new file, and never part of anything else. What they are
//! for is a copy to give away.
//!
//! Neither touches a block it cannot account for. A file's slack is wiped
//! only when its tables describe exactly the blocks its size needs, and only
//! in a last block no other file also uses; on OFS, only when that block
//! carries the file's own header key and a valid checksum, which is resealed.
//! A free block is wiped only when the caller has established that nothing
//! uses it — the layer above passes the set its health report is built on.

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

use ade_block::{BlockIndex, Geometry, checksum};
use ade_endian::{put_u32, u32_at};

use crate::{
    bitmap::Bitmap,
    dostype::FileSystem,
    edit::{EditError, Slice, apply},
    entry::{EntryKind, T_DATA},
    volume::{FsError, Volume},
};

/// What a wipe did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wiped {
    /// Blocks written.
    pub blocks: usize,
    /// Bytes that were not already zero.
    pub bytes: u64,
    /// Files whose slack was left, each with why.
    pub skipped: Vec<String>,
}

/// Zero every file's slack: the bytes after its last one, in its last block.
///
/// # Errors
/// [`EditError::Fs`] when the volume cannot be mounted or walked. A file that
/// cannot be accounted for is skipped and listed, not an error. The image is
/// unchanged whenever one is returned.
pub fn slack(image: &mut [u8], geometry: Geometry) -> Result<Wiped, EditError> {
    let (wiped, writes) = {
        let source = Slice { image, geometry };
        let volume = Volume::mount(&source)?;
        let ofs = volume
            .dostype()
            .is_some_and(|d| d.filesystem() == FileSystem::Ofs);
        let bsize = geometry.block_size() as usize;
        let capacity = if ofs { bsize.saturating_sub(24) } else { bsize };
        let walk = volume.walk(volume.root())?;
        let files: Vec<_> = walk
            .entries
            .iter()
            .filter(|(_, e)| e.kind == EntryKind::File)
            .collect();

        // How many files use each block: a shared one is nobody's to wipe.
        let mut users: HashMap<u32, usize> = HashMap::new();
        for (_, e) in &files {
            for b in volume.file_blocks(e).unwrap_or_default() {
                let n = users.entry(b).or_insert(0);
                *n = n.saturating_add(1);
            }
        }

        let mut wiped = Wiped::default();
        let mut writes = std::collections::BTreeMap::new();
        for (path, e) in files {
            let size = e.byte_size as usize;
            if size == 0 {
                continue;
            }
            let data = volume.data_blocks(e)?;
            let needed = size.div_ceil(capacity.max(1));
            if data.len() != needed {
                wiped.skipped.push(format!(
                    "{path}: its tables list {} data blocks where {size} bytes need {needed}",
                    data.len()
                ));
                continue;
            }
            let Some(&last) = data.last() else { continue };
            if users.get(&last).copied().unwrap_or(0) > 1 {
                wiped.skipped.push(format!(
                    "{path}: its last block, {last}, another file uses too"
                ));
                continue;
            }
            let used = size.saturating_sub(capacity.saturating_mul(needed.saturating_sub(1)));
            let mut raw = read(&source, last)?;
            let from = if ofs {
                let owner = u32_at(&raw, 4).unwrap_or(0);
                let stored = u32_at(&raw, 12).unwrap_or(0) as usize;
                if u32_at(&raw, 0).unwrap_or(0) != T_DATA
                    || owner != e.block
                    || stored != used
                    || !checksum::normal_valid(&raw)
                {
                    wiped.skipped.push(format!(
                        "{path}: its last block, {last}, is not the data block its header \
                         says, or fails its checksum"
                    ));
                    continue;
                }
                24usize.saturating_add(used)
            } else {
                used
            };
            let tail = raw.get_mut(from..).unwrap_or(&mut []);
            let dirty = tail.iter().filter(|&&b| b != 0).count();
            if dirty == 0 {
                continue;
            }
            tail.fill(0);
            if ofs {
                let _ = put_u32(&mut raw, 20, 0);
                if let Some(sum) = checksum::normal_at(&raw, 20) {
                    let _ = put_u32(&mut raw, 20, sum);
                }
            }
            wiped.blocks = wiped.blocks.saturating_add(1);
            wiped.bytes = wiped.bytes.saturating_add(dirty as u64);
            writes.insert(last, raw);
        }
        (wiped, writes)
    };
    apply(image, geometry, &writes);
    Ok(wiped)
}

/// Zero every block the bitmap marks free and `in_use` does not contain.
///
/// `in_use` is every block the volume's tree reaches; the rootblock and the
/// bitmap's own blocks are never written. Blocks marked used that nothing
/// reaches are left: they are not free, and may be a trackloader's.
///
/// # Errors
/// [`EditError::BitmapUntrusted`] when the bitmap is flagged invalid, fails a
/// checksum, does not cover the volume or points outside it: a free map ADE
/// distrusts is how a wipe lands on live data. The image is unchanged
/// whenever one is returned.
pub fn free_space<S: BuildHasher>(
    image: &mut [u8],
    geometry: Geometry,
    in_use: &HashSet<u32, S>,
) -> Result<Wiped, EditError> {
    let (wiped, writes) = {
        let source = Slice { image, geometry };
        let volume = Volume::mount(&source)?;
        let bitmap = Bitmap::read(&source, &geometry, volume.rootblock())?;
        if !bitmap.flagged_valid
            || !bitmap.bad_checksums.is_empty()
            || bitmap.incomplete
            || bitmap.blocks.is_empty()
            || !bitmap.out_of_range.is_empty()
        {
            return Err(EditError::BitmapUntrusted(
                "the bitmap is flagged invalid, fails a checksum or does not describe the \
                 volume, so it cannot say which blocks are free; repair the disk first"
                    .to_owned(),
            ));
        }
        let bsize = geometry.block_size() as usize;
        let zero = vec![0u8; bsize];
        let mut wiped = Wiped::default();
        let mut writes = std::collections::BTreeMap::new();
        for b in u64::from(geometry.reserved())..geometry.total_blocks() {
            let Ok(b) = u32::try_from(b) else { break };
            if bitmap.is_allocated(b)
                || in_use.contains(&b)
                || b == volume.root()
                || bitmap.blocks.contains(&b)
            {
                continue;
            }
            let raw = read(&source, b)?;
            let dirty = raw.iter().filter(|&&x| x != 0).count();
            if dirty > 0 {
                wiped.blocks = wiped.blocks.saturating_add(1);
                wiped.bytes = wiped.bytes.saturating_add(dirty as u64);
                writes.insert(b, zero.clone());
            }
        }
        (wiped, writes)
    };
    apply(image, geometry, &writes);
    Ok(wiped)
}

fn read(source: &Slice<'_>, block: u32) -> Result<Vec<u8>, EditError> {
    let mut raw = vec![0u8; source.geometry.block_size() as usize];
    ade_block::read_at(source, BlockIndex(u64::from(block)), &mut raw).map_err(FsError::Block)?;
    Ok(raw)
}
