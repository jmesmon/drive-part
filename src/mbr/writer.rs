use std::{time};
use io_block::{BlockSize};
use io_at;
use io_at::{WriteAt};

/// Identify another partition by it's relative or absolute index
#[derive(Clone,PartialEq,Eq,Debug)]
pub enum PartRef {
    /** N partitions before this one. 0 is the current partition. 1 is the one immediately
     * previous. */
    Previous(u32),

    /** N partitions after this one. 0 is the one immediately after. */
    Next(u32),

    /** Partition with number N. 0 is the first partition (all partitions are numbered from 0. */
    Exact(u32),
}

/// "Partition edge should be located [X]"
#[derive(Clone,PartialEq,Eq,Debug)]
pub enum LocSpec {
    /** At the end of a partition */
    AtEndOf(PartRef),

    /** At the start of a partition */
    AtStartOf(PartRef),

    /*
    /** Offset by N bytes from another location */
    pub Offset(LocSpec, i64),

    /** Align the location rounding to the next location divisible by N bytes */
    pub AlignNext(LocSpec, u64),

    /** Align the location rounding to the previous location divisible by N bytes */
    pub AlignPrev(LocSpec, u64),
    */
}

/// "Partition index should be [X]"
#[derive(Clone,PartialEq,Eq,Debug)]
pub enum NumSpec {
    Exact(u32),
    AfterPart(PartRef),
    BeforePart(PartRef),
}

/// Requirements that can be applied to a given partition
#[derive(Clone,PartialEq,Eq,Debug)]
pub enum PartSpec {
    Number(NumSpec),
    Start(LocSpec),
    End(LocSpec),
    IsBootable
}

/// Each partition spec (aka request) supplies a series of constraints that should be satisfied by
/// the concrete (relealized, actual) partition. Convertion to a real partition is handled by
/// `MbrBuilder::compile()`.
#[derive(Clone,PartialEq,Eq,Debug)]
pub struct MbrPartSpec {
    specs: Vec<PartSpec>,
}

impl MbrPartSpec {
    pub fn is_bootable(&self) -> bool {
        for s in self.specs.iter() {
            if let &PartSpec::IsBootable = s {
                return true;
            }
        }
        false
    }
}

/// A physical (real) MBR partition with all associated attributes
#[derive(Clone)]
pub struct MbrPhysPart {
    number: u32,
    start: u64,
    end: u64,
    bootable: bool,
}

impl MbrPhysPart {
    pub fn is_primary(&self) -> bool {
        self.number < 4
    }

    pub fn is_extended(&self) -> bool {
        !self.is_primary()
    }
}

#[derive(Clone,PartialEq,Eq)]
pub enum MbrBuilderError {
    BootcodeOversized(usize),
    Bootcode2Oversized(usize),
    OriginalPhysDriveOverlapped,
    DiskSigOverlapped,
    BootCodeOverlapped(usize, usize),
    MoreThan1Bootable,
}

/// Allows creating and commiting a new MBR to a WriteAt-able BlockSize-able thing (typically, a
/// block device).
#[derive(Clone)]
pub struct MbrBuilder {
    bootcode: Option<Vec<u8>>,
    bootcode_2: Option<Vec<u8>>,
    partitions: Vec<MbrPartSpec>,
    timestamp: Option<time::SystemTime>,
    original_physical_drive: Option<u8>,
    disk_sig: Option<(u32,u16)>,
}

impl MbrBuilder {
    // TODO: consider determining presense of data prior to writing
    pub fn new() -> Self {
        MbrBuilder {
            bootcode: None,
            bootcode_2: None,
            partitions: vec![],
            timestamp: None,
            original_physical_drive: None,
            disk_sig: None
        }
    }

    /// MBR contains a block of "bootcode" that is 446 bytes long in classic MBR or 226 bytes long
    /// in modern MBR (for the first half of it).
    ///
    /// This function lets you set the bootcode. Slices less than 446 bytes will be padded with
    /// zeros (this may not be ideal consider carefully).
    ///
    /// Panics:
    ///
    ///  - if code.len() is too long for the type of MBR being constructed.
    pub fn set_bootcode(mut self, code: &[u8]) -> Self {
        if code.len() > 446 {
            panic!("Bootcode must be at most 446 bytes long, got {} bytes", code.len())
        }

        self.bootcode = Some(code.to_owned());
        self
    }

    /// In place of some of the bootcode, modern MBR can contain a disk timestamp (seconds,
    /// minutes, hours). The same space may alternately be populated by a OEM loader signature with
    /// NEWLDR.
    ///
    /// This is entirely optional (and probably unlikely to be used
    pub fn set_timestamp(mut self, ts: time::SystemTime) -> Self {
        self.timestamp = Some(ts);
        self
    }

    /// Considered a piece of the timestamp set by `set_timestamp()`
    ///
    /// `drv` is intended to be a BIOS drive number (0x80 to 0xFF).
    pub fn set_original_physical_drive(mut self, drv: u8) -> Self {
        self.original_physical_drive = Some(drv);
        self
    }

    /// In modern MBR, bootcode is split into 2 pieces: 1x226 bytes at byte 0, and 1x216 (or 1x222)
    /// at +224 bytes.
    ///
    /// This sets the second part of the bootcode.
    pub fn set_bootcode_part2(mut self, code: &[u8]) -> Self {
        if code.len() > 222 {
            panic!("Bootcode #2 must be at most 222 bytes long, was {} bytes", code.len());
        }
        self.bootcode_2 = Some(code.to_owned());
        self
    }

    /// An optional component of the partition table.
    ///
    /// TODO: note the format of `sig` here
    ///
    /// `extra` is normally 0x0000, but may be 0x5A5A to mark the disk as copy protected.
    ///
    /// Adding this element shrinks the 2nd bootcode part (`set_bootcode_part2()`) as it occupies
    /// space at bootcode_part2's end.
    pub fn set_disk_signature(mut self, sig: u32, extra: u16) -> Self {
        self.disk_sig = Some((sig, extra));
        self
    }

    /// Add a partition by specification
    pub fn partition_add(mut self, spec: MbrPartSpec) -> Self {
        self.partitions.push(spec);
        self
    }

    fn is_modern(&self) -> bool {
        self.bootcode_2.is_some() ||
            self.original_physical_drive.is_some() ||
            self.timestamp.is_some() ||
            self.disk_sig.is_some()
    }

    fn partition_check(&self) -> Result<(),MbrBuilderError> {
        let mut fb = false;
        for p in self.partitions.iter() {
            /* only 1 bootable partition is allowed */
            if p.is_bootable() {
                if fb {
                    return Err(MbrBuilderError::MoreThan1Bootable)
                }
                fb = true;
            }
        }

        Ok(())
    }

    /// Confirm that the MBR specified by our building is buildable, and convert it into a
    /// MbrWriter which may be used to commit the MBR to disk
    pub fn compile(self) -> Result<MbrWriter, MbrBuilderError> {
        let b1 = self.bootcode.as_ref().map_or(0, |x| x.len());
        let b2 = self.bootcode_2.as_ref().map_or(0, |x| x.len());

        if self.original_physical_drive.is_some() && b1 > 218 {
            return Err(MbrBuilderError::OriginalPhysDriveOverlapped)
        }

        if self.timestamp.is_some() && b1 > 221 {
            return Err(MbrBuilderError::BootcodeOversized(b1));
        }

        if self.disk_sig.is_some() && (b1 > 440 || b2 > 216) {
            return Err(MbrBuilderError::DiskSigOverlapped);
        }

        if b2 > 0 && b1 > 224 {
            return Err(MbrBuilderError::BootCodeOverlapped(b1, b2));
        }

        /* TODO: confirm that partition specification is valid */

        Ok(MbrWriter { inner: self })
    }
}

/// A MBR specification that may be directly commited to a device.
pub struct MbrWriter {
    inner: MbrBuilder,
}

impl MbrWriter {
    /// This mbr has modern features included in it.
    pub fn is_modern(&self) -> bool {
        self.inner.is_modern()
    }

    /// Commit the MBR we've built up here to a backing store.
    ///
    /// Note that no attempt to preseve the existing contents of the backing store will be made by
    /// _this_ function. Preservation is handled elsewhere by pre-configuring the builder.
    ///
    /// It is recommended that you ensure no unintended changes are made between read & commit.
    pub fn commit<T: WriteAt + BlockSize>(&self, back: T) -> io_at::Result<()> {
        /* 1. Confirm that given the size of the device, the requested partition specs result in an
         *    allowed layout (ie: they need to fit)
         */


        unimplemented!();
        Ok(())
    }
}
