//! Editing a partitioned disk's table (F-038, D-024 part 5).
//!
//! **Surgical, never regenerated.** A real disk's RDB can carry a filesystem
//! in `FSHD`/`LSEG` blocks, a bad-block list, a drive's own strings and
//! controller values HDToolBox chose; rewriting the table from what ADE
//! understands would lose them. So an edit changes the fields it is about —
//! a drive name, the bootable flag, a boot priority, a dostype, a chain link
//! — in the blocks that hold them, reseals those blocks, and leaves every
//! other byte. A new `PART` goes in a block of the table's own reserved
//! range that no list uses.
//!
//! Each edit is one step of history, kept only if the table still passes
//! every check D-024 part 3 makes and any partition it formatted examines
//! clean; otherwise it is put back. Undoing one reads the partitions again.

use std::collections::HashMap;

use ade_block::Geometry;
use ade_filesystem::dostype::Dostype;
use ade_filesystem::edit::{Draft, Writes};
use ade_filesystem::rdb::{self, END_OF_LIST, NewPartition, Partition, RigidDiskBlock};

use super::{
    AreaState, EditError, FreeData, Session, Step, device_areas, quick_format_writes, table_faults,
};
use crate::create::PartitionSize;
use crate::health::{self, Severity};

/// The table as it stands: the RDB, its partitions in chain order, and every
/// block its lists occupy.
struct Table {
    rdb: RigidDiskBlock,
    parts: Vec<Partition>,
    used: std::collections::HashSet<u32>,
}

/// A refusal of a table edit that was never a disk: a name, a size, a
/// partition that is not there.
fn refuse(why: impl Into<String>) -> EditError {
    EditError::NotEditable(why.into())
}

impl Session {
    /// The image as a run of 512-byte blocks, as the RDB addresses it.
    fn device_geometry(&self) -> Result<Geometry, EditError> {
        let blocks = u32::try_from(self.bytes.len() / 512)
            .map_err(|_| refuse("the disk is too large to address"))?;
        Geometry::new(blocks, 1, 1, 512, 2).map_err(|e| refuse(e.to_string()))
    }

    fn read_table(&self) -> Result<Table, EditError> {
        if !self.is_partitioned() {
            return Err(refuse("this image has no partition table"));
        }
        let geometry = self.device_geometry()?;
        let view = Draft::new(&self.bytes, geometry);
        let rdb = RigidDiskBlock::find(&view, &geometry)
            .map_err(EditError::Lookup)?
            .ok_or_else(|| refuse("this image has no partition table"))?;
        let (parts, faults) = rdb::read_partitions(&view, &geometry, &rdb);
        if let Some(fault) = faults.first() {
            return Err(refuse(format!(
                "the partition table cannot be read: {fault}"
            )));
        }
        let used = rdb::table_block_usage(&view, &geometry, &rdb);
        Ok(Table { rdb, parts, used })
    }

    /// A copy of device block `block`.
    fn device_block(&self, block: u32) -> Result<Vec<u8>, EditError> {
        let at = (block as usize).saturating_mul(512);
        self.bytes
            .get(at..at.saturating_add(512))
            .map(<[u8]>::to_vec)
            .ok_or_else(|| refuse(format!("block {block} is past the end of the disk")))
    }

    /// The partition `wanted` names — by drive name, ignoring case, or by
    /// index — as an index into the table.
    fn table_index(table: &Table, wanted: &str) -> Result<usize, EditError> {
        table
            .parts
            .iter()
            .position(|p| p.name_lossy().eq_ignore_ascii_case(wanted))
            .or_else(|| {
                wanted
                    .parse::<usize>()
                    .ok()
                    .filter(|i| *i < table.parts.len())
            })
            .ok_or_else(|| {
                let names: Vec<String> = table.parts.iter().map(Partition::name_lossy).collect();
                refuse(format!(
                    "no partition {wanted:?}; this device has {}",
                    if names.is_empty() {
                        "none".to_owned()
                    } else {
                        names.join(", ")
                    }
                ))
            })
    }

    /// A drive name ADE will write, and not one another partition has.
    fn new_drive_name(
        table: &Table,
        name: &str,
        except: Option<usize>,
    ) -> Result<Vec<u8>, EditError> {
        let bytes = crate::create::drive_name(name).map_err(|e| refuse(e.to_string()))?;
        let taken = table
            .parts
            .iter()
            .enumerate()
            .any(|(i, p)| Some(i) != except && p.name.eq_ignore_ascii_case(&bytes));
        if taken {
            return Err(refuse(format!("another partition is already named {name}")));
        }
        Ok(bytes)
    }

    /// Rename partition `wanted` to `name`: its drive name, the one AmigaDOS
    /// mounts it as (`DH1:`). Its volume keeps its own name.
    ///
    /// # Errors
    /// [`EditError::NotEditable`] for a partition that is not there, or a
    /// name that is illegal or taken; [`EditError::WouldDamage`] should the
    /// result fail the table's checks.
    pub fn rename_partition(&mut self, wanted: &str, name: &str) -> Result<(), EditError> {
        let table = self.read_table()?;
        let index = Self::table_index(&table, wanted)?;
        let old_name = table
            .parts
            .get(index)
            .map(Partition::name_lossy)
            .unwrap_or_default();
        let bytes = Self::new_drive_name(&table, name, Some(index))?;
        let part = table.parts.get(index).map(|p| p.block).unwrap_or_default();
        let mut block = self.device_block(part)?;
        // DriveName is a BSTR in 32 bytes: its length, then its characters.
        if let Some(field) = block.get_mut(0x24..0x44) {
            field.fill(0);
        }
        if let Some(len) = block.get_mut(0x24) {
            *len = u8::try_from(bytes.len()).unwrap_or(31);
        }
        if let Some(field) = block.get_mut(0x25..0x25usize.saturating_add(bytes.len())) {
            field.copy_from_slice(&bytes);
        }
        rdb::reseal(&mut block);
        // The selection follows the partition under its new name.
        let renamed_current = self
            .partition()
            .is_some_and(|c| c.eq_ignore_ascii_case(&old_name));
        let prefer = if renamed_current {
            Some(name.to_owned())
        } else {
            self.partition().map(str::to_owned)
        };
        self.commit_table(&Writes::from([(part, block)]), None, prefer.as_deref())
    }

    /// Mark partition `wanted` bootable at `priority`, or — `None` — not
    /// bootable. Kickstart boots the bootable partition with the highest
    /// priority; -128 is never.
    ///
    /// # Errors
    /// As [`Self::rename_partition`], and for a partition whose environment
    /// vector is too short to hold a boot priority.
    pub fn set_partition_boot(
        &mut self,
        wanted: &str,
        priority: Option<i32>,
    ) -> Result<(), EditError> {
        let table = self.read_table()?;
        let index = Self::table_index(&table, wanted)?;
        let old_name = table
            .parts
            .get(index)
            .map(Partition::name_lossy)
            .unwrap_or_default();
        let part = table.parts.get(index).map(|p| p.block).unwrap_or_default();
        let mut block = self.device_block(part)?;
        let flags = ade_endian::u32_at(&block, 0x14).unwrap_or(0);
        let _ = ade_endian::put_u32(
            &mut block,
            0x14,
            if priority.is_some() {
                flags | 1
            } else {
                flags & !1
            },
        );
        if let Some(priority) = priority {
            // DE_BOOTPRI is the vector's 15th entry: a shorter vector has none.
            if ade_endian::u32_at(&block, 0x80).unwrap_or(0) < 15 {
                return Err(refuse(format!(
                    "{old_name}'s environment vector is too short to hold a boot priority"
                )));
            }
            let _ = ade_endian::put_i32(&mut block, 0xbc, priority);
        }
        rdb::reseal(&mut block);
        let prefer = self.partition().map(str::to_owned);
        self.commit_table(&Writes::from([(part, block)]), None, prefer.as_deref())
    }

    /// Delete partition `wanted`: it leaves the table, and its volume goes
    /// with it — the blocks stay as they were until something is written
    /// over them, and Undo puts the partition back.
    ///
    /// # Errors
    /// As [`Self::rename_partition`].
    pub fn delete_partition(&mut self, wanted: &str) -> Result<(), EditError> {
        let table = self.read_table()?;
        let index = Self::table_index(&table, wanted)?;
        let old_name = table
            .parts
            .get(index)
            .map(Partition::name_lossy)
            .unwrap_or_default();
        let gone = table.parts.get(index).map(|p| p.block).unwrap_or_default();
        let after = ade_endian::u32_at(&self.device_block(gone)?, 0x10).unwrap_or(END_OF_LIST);
        // Whatever pointed at it points past it: the RDB's list head, or the
        // partition before.
        let (from, field) = match index.checked_sub(1).and_then(|i| table.parts.get(i)) {
            Some(before) => (before.block, 0x10),
            None => (table.rdb.block, 0x1c),
        };
        let mut link = self.device_block(from)?;
        let _ = ade_endian::put_u32(&mut link, field, after);
        rdb::reseal(&mut link);
        let writes = Writes::from([(from, link), (gone, vec![0u8; 512])]);
        let deleting_current = self
            .partition()
            .is_some_and(|c| c.eq_ignore_ascii_case(&old_name));
        let prefer = if deleting_current {
            None
        } else {
            self.partition().map(str::to_owned)
        };
        self.commit_table(&writes, None, prefer.as_deref())
    }

    /// Add a partition called `name` of `size` in the first unused cylinders
    /// that hold it — `Rest` takes the largest run — formatted with
    /// `filesystem` and named `volume` (the drive name when `None`). Not
    /// bootable. Returns its index in the table.
    ///
    /// # Errors
    /// As [`Self::rename_partition`], and for a size no unused run of
    /// cylinders holds, or a table with no free block for another `PART`.
    pub fn add_partition(
        &mut self,
        name: &str,
        size: PartitionSize,
        filesystem: Dostype,
        volume: Option<&str>,
    ) -> Result<usize, EditError> {
        let table = self.read_table()?;
        let drive = Self::new_drive_name(&table, name, None)?;
        let rdb = &table.rdb;
        let per = rdb.heads.saturating_mul(rdb.sectors);
        if per == 0 || !rdb.geometry_consistent() {
            return Err(refuse(
                "the table's heads, sectors and blocks per cylinder disagree, so a new \
                 partition's cylinders cannot be placed",
            ));
        }

        let runs = self.free_cylinders(&table, per);
        let (first, count) = match size {
            PartitionSize::Rest => runs.iter().copied().max_by_key(|r| r.1),
            PartitionSize::Megabytes(m) => {
                let wanted =
                    u32::try_from(u64::from(m).saturating_mul(2048).div_ceil(u64::from(per)))
                        .unwrap_or(u32::MAX);
                runs.iter()
                    .copied()
                    .find(|r| r.1 >= wanted)
                    .map(|r| (r.0, wanted))
            }
        }
        .ok_or_else(|| {
            let largest = runs.iter().map(|r| r.1).max().unwrap_or(0);
            refuse(format!(
                "no unused run of cylinders holds that; the largest is {} MB",
                u64::from(largest)
                    .saturating_mul(u64::from(per))
                    .checked_div(2048)
                    .unwrap_or(0)
            ))
        })?;

        let part = Self::free_part_block(&table, per)?;
        let mut writes = Writes::new();
        let new = NewPartition {
            name: drive.clone(),
            low_cylinder: first,
            high_cylinder: first.saturating_add(count).saturating_sub(1),
            dostype: filesystem.raw(),
            bootable: false,
            boot_priority: 0,
        };
        writes.insert(
            part,
            rdb::part_block(rdb.heads, rdb.sectors, &new, END_OF_LIST).to_vec(),
        );
        // Chained at the end of the list, and the RDB's high-water mark moved.
        let mut rdsk = self.device_block(rdb.block)?;
        if let Some(tail) = table.parts.last() {
            let mut link = self.device_block(tail.block)?;
            let _ = ade_endian::put_u32(&mut link, 0x10, part);
            rdb::reseal(&mut link);
            writes.insert(tail.block, link);
        } else {
            let _ = ade_endian::put_u32(&mut rdsk, 0x1c, part);
        }
        let high = ade_endian::u32_at(&rdsk, 0x98).unwrap_or(0);
        let _ = ade_endian::put_u32(&mut rdsk, 0x98, high.max(part));
        rdb::reseal(&mut rdsk);
        writes.insert(rdb.block, rdsk);

        // Its volume, formatted in place.
        let geometry = Geometry::new(count, rdb.heads, rdb.sectors, 512, 2)
            .map_err(|e| refuse(e.to_string()))?;
        let label = volume.unwrap_or(name);
        let label: Vec<u8> = label
            .chars()
            .map(|c| u8::try_from(u32::from(c)))
            .collect::<Result<_, _>>()
            .map_err(|_| refuse(format!("{label:?}: an Amiga name is Latin-1")))?;
        let start = first.saturating_mul(per);
        for (block, raw) in quick_format_writes(geometry, filesystem, &label, self.now())? {
            writes.insert(start.saturating_add(block), raw);
        }
        let prefer = self
            .partition()
            .map_or_else(|| name.to_owned(), str::to_owned);
        self.commit_table(&writes, Some(name), Some(&prefer))?;
        Ok(table.parts.len())
    }

    /// Unused runs of whole cylinders within the RDB's partition range and
    /// the disk, as (first cylinder, count), in order.
    fn free_cylinders(&self, table: &Table, per: u32) -> Vec<(u32, u32)> {
        let rdb = &table.rdb;
        let per64 = u64::from(per.max(1));
        let disk_cylinders = u32::try_from(
            (self.bytes.len() as u64)
                .checked_div(512u64.saturating_mul(per64))
                .unwrap_or(0),
        )
        .unwrap_or(u32::MAX);
        let last = rdb.hi_cylinder.min(disk_cylinders.saturating_sub(1));
        let mut taken: Vec<(u64, u64)> = table
            .parts
            .iter()
            .map(|p| {
                (
                    p.first_block(),
                    p.first_block().saturating_add(p.block_count()),
                )
            })
            .collect();
        taken.sort_unstable();
        let mut runs = Vec::new();
        let mut cylinder = rdb.lo_cylinder;
        for (start, end) in taken.iter().copied().chain([(u64::MAX, u64::MAX)]) {
            let start_cyl =
                u32::try_from(start.checked_div(per64).unwrap_or(0)).unwrap_or(u32::MAX);
            let until = start_cyl.min(last.saturating_add(1));
            if until > cylinder {
                runs.push((cylinder, until.saturating_sub(cylinder)));
            }
            if end == u64::MAX {
                break;
            }
            cylinder = cylinder.max(u32::try_from(end.div_ceil(per64)).unwrap_or(u32::MAX));
        }
        runs
    }

    /// A block for a new `PART`: in the table's own reserved range, before
    /// the first partition cylinder, used by no list.
    fn free_part_block(table: &Table, per: u32) -> Result<u32, EditError> {
        let rdb = &table.rdb;
        let reserved_end = rdb.lo_cylinder.saturating_mul(per);
        let range_end = if rdb.rdb_blocks_hi > rdb.block {
            rdb.rdb_blocks_hi.min(reserved_end.saturating_sub(1))
        } else {
            reserved_end.saturating_sub(1)
        };
        (rdb.block.saturating_add(1)..=range_end)
            .find(|b| !table.used.contains(b))
            .ok_or_else(|| refuse("the partition table has no free block for another partition"))
    }

    /// The selected partition reformatted with a new filesystem: its volume's
    /// `writes`, at the partition's place on the disk, and its table entry's
    /// dostype, as one step.
    pub(super) fn reformat_partition(
        &mut self,
        writes: &Writes,
        dostype: Dostype,
    ) -> Result<(), EditError> {
        let Some(current) = self.partition().map(str::to_owned) else {
            return Err(refuse("no partition is selected"));
        };
        let table = self.read_table()?;
        let index = Self::table_index(&table, &current)?;
        let part = table.parts.get(index).map(|p| p.block).unwrap_or_default();
        let mut block = self.device_block(part)?;
        // DE_DOSTYPE is the vector's 16th entry: a shorter vector has none,
        // and AmigaDOS would mount the partition as OFS whatever was written.
        if ade_endian::u32_at(&block, 0x80).unwrap_or(0) < 16 {
            return Err(refuse(format!(
                "{current}'s environment vector is too short to hold a dostype"
            )));
        }
        let _ = ade_endian::put_u32(&mut block, 0xc0, dostype.raw());
        rdb::reseal(&mut block);
        let start = u32::try_from(self.offset / 512).unwrap_or(u32::MAX);
        let mut all: Writes = writes
            .iter()
            .map(|(b, raw)| (start.saturating_add(*b), raw.clone()))
            .collect();
        all.insert(part, block);
        self.commit_table(&all, Some(&current), Some(&current))
    }

    /// Make a table edit's `writes` — whole 512-byte device blocks — in
    /// place, and keep them only if the table still passes D-024 part 3's
    /// checks and the partition `fresh` names, which the edit formatted,
    /// examines clean. Otherwise they are put back. Kept, they are one step
    /// of history, and the partitions are read again with `prefer` selected
    /// when it still holds a volume.
    fn commit_table(
        &mut self,
        writes: &Writes,
        fresh: Option<&str>,
        prefer: Option<&str>,
    ) -> Result<(), EditError> {
        let mut step = Step::make(&mut self.bytes, writes, 512, (0, 0));
        step.areas.clear();
        step.table = true;
        let (inspection, bytes) =
            crate::inspect::inspect_returning(std::mem::take(&mut self.bytes));
        self.bytes = bytes;
        let mut faults = table_faults(&inspection, self.bytes.len() as u64);
        if let Some(name) = fresh {
            let (report, bytes) =
                health::examine_partition_returning(std::mem::take(&mut self.bytes), Some(name));
            self.bytes = bytes;
            if report.examined.is_none() {
                faults.push(format!(
                    "{name} holds no volume ADE can mount once formatted"
                ));
            }
            // What was on the disk before is not damage the format did.
            faults.extend(
                report
                    .findings
                    .iter()
                    .filter(|f| {
                        f.severity >= Severity::Warning && f.code != "free-blocks-hold-data"
                    })
                    .map(|f| format!("{}: {}", f.code, f.message)),
            );
        }
        if !faults.is_empty() {
            step.apply(&mut self.bytes);
            return Err(EditError::WouldDamage(faults));
        }
        match self.undo.last_mut() {
            Some(open) if self.group_depth > 0 && self.group_snapshot => open.absorb(step),
            _ => {
                self.push_undo(step);
                self.group_snapshot = self.group_depth > 0;
            }
        }
        self.redo.clear();
        self.dirty = true;
        self.rebuild_areas(prefer);
        Ok(())
    }

    /// Read the partitions again after the table changed. A partition that
    /// is where it was, the same size and name, keeps what was known of it —
    /// its baseline, and a consent to overwrite free data; the selection
    /// goes to `prefer` if it holds a volume, else to the first that does,
    /// else to none.
    pub(super) fn rebuild_areas(&mut self, prefer: Option<&str>) {
        let (inspection, bytes) =
            crate::inspect::inspect_returning(std::mem::take(&mut self.bytes));
        self.bytes = bytes;
        let mut areas = device_areas(&inspection);
        let live = self.state();
        let old = std::mem::take(&mut self.areas);
        for area in &mut areas {
            let same = old.iter().enumerate().find(|(_, o)| {
                o.name == area.name && o.offset == area.offset && o.geometry == area.geometry
            });
            if let Some((i, o)) = same {
                area.saved = if i == self.current {
                    Some(live.clone())
                } else {
                    o.saved.clone()
                };
            }
        }
        let named = |a: &super::Area| {
            a.mounts
                && prefer
                    .is_some_and(|p| a.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(p)))
        };
        // By name; else where the selected partition was — a rename, or an
        // undone one, changes the name and nothing else; else the first.
        let was = old.get(self.current).map(|a| (a.offset, a.geometry));
        let pick = areas
            .iter()
            .position(named)
            .or_else(|| {
                areas
                    .iter()
                    .position(|a| a.mounts && Some((a.offset, a.geometry)) == was)
            })
            .or_else(|| areas.iter().position(|a| a.mounts));
        self.areas = areas;
        let Some(index) = pick else {
            self.current = self.areas.len();
            self.geometry = Geometry::DD_FLOPPY;
            self.offset = 0;
            self.load(&AreaState {
                baseline: HashMap::new(),
                unsound: None,
                free_holding_data: 0,
                free_data: FreeData::Unprotected,
            });
            return;
        };
        self.current = index;
        let saved = self.areas.get_mut(index).and_then(|a| a.saved.take());
        let state = if let Some(state) = saved {
            state
        } else {
            let report = self.examine(index);
            AreaState::of(&report, None)
        };
        if let Some(area) = self.areas.get(index) {
            self.geometry = area.geometry;
            self.offset = area.offset;
        }
        self.load(&state);
    }
}
