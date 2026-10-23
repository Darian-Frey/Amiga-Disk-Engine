//! What an address means on an Amiga (F-032).
//!
//! A program that talks to the hardware does it through fixed addresses, and
//! a listing that prints `$dff096` tells almost nobody that the program is
//! switching DMA channels. This names them: the custom-chip registers at
//! `$dff000`, the two CIAs, the 68000's exception vectors in low memory, and
//! the one pointer every Amiga program reads, ExecBase at `$4`.
//!
//! Sources: the *Amiga Hardware Reference Manual* (3rd ed.), Appendix A
//! (register summary by address) and the CIA chapter for the port bits; the
//! *M68000 Programmer's Reference Manual* for the vector table. ECS and AGA
//! registers are included and marked, because an OCS machine ignores them and
//! a program writing one says something about the machine it expected.
//!
//! An address not in these tables gets no name. A guess at what `$1000` is
//! would be worse than silence: it is ordinary chip memory, and a program may
//! put anything there.

/// A named address: the register's name and what it is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Named {
    /// The name the Hardware Reference Manual uses: `DMACON`.
    pub name: &'static str,
    /// What it does, in a few words.
    pub meaning: &'static str,
}

/// The name of the hardware at `address`, if it has one.
#[must_use]
pub fn name(address: u32) -> Option<Named> {
    custom(address)
        .or_else(|| cia(address))
        .or_else(|| vector(address))
}

/// What an address is as a *base*: the start of a block of registers a
/// program addresses relative to. `lea $dff000,a6` loads the custom chips'
/// base so every register is `n(a6)` — naming that address after the register
/// that happens to sit at offset 0 would be wrong.
#[must_use]
pub fn base(address: u32) -> Option<&'static str> {
    match address {
        CUSTOM => Some("the base of the custom chip registers"),
        0x00BF_E001 => Some("the base of CIA-A's registers"),
        0x00BF_D000 => Some("the base of CIA-B's registers"),
        _ => None,
    }
}

/// The base of the custom chips' registers.
pub const CUSTOM: u32 = 0x00DF_F000;

/// The custom-chip register at `address`, by its offset from `$dff000`.
#[allow(
    clippy::too_many_lines,
    reason = "one arm per register, in the order of the Hardware Reference Manual's table"
)]
fn custom(address: u32) -> Option<Named> {
    let offset = address.checked_sub(CUSTOM)?;
    // Every custom register is a word at an even offset.
    if offset > 0x1FE || offset % 2 != 0 {
        return None;
    }
    let n = |name, meaning| Some(Named { name, meaning });
    // Registers that repeat: audio channels, bitplane and sprite pointers,
    // sprite control, colours. Handled by arithmetic rather than by listing
    // sixty-four nearly identical lines.
    match offset {
        0xA0..=0xDF => return audio(offset),
        0xE0..=0xFF => return bitplane_pointer(offset),
        0x110..=0x11F => return n(BPLDAT.get(index(offset, 0x110, 2)?)?, "bitplane data"),
        0x120..=0x13F => return sprite_pointer(offset),
        0x140..=0x17F => return sprite_control(offset),
        0x180..=0x1BF => return colour(offset),
        _ => {}
    }
    match offset {
        0x000 => n("BLTDDAT", "blitter destination data (read)"),
        0x002 => n("DMACONR", "DMA control and blitter status (read)"),
        0x004 => n("VPOSR", "beam vertical position, high bit (read)"),
        0x006 => n("VHPOSR", "beam position, vertical and horizontal (read)"),
        0x008 => n("DSKDATR", "disk data (read)"),
        0x00A => n("JOY0DAT", "mouse/joystick counters, port 1"),
        0x00C => n("JOY1DAT", "mouse/joystick counters, port 2"),
        0x00E => n("CLXDAT", "collision detection (read)"),
        0x010 => n("ADKCONR", "audio and disk control (read)"),
        0x012 => n("POT0DAT", "potentiometer counters, port 1"),
        0x014 => n("POT1DAT", "potentiometer counters, port 2"),
        0x016 => n("POTGOR", "pot port data and right mouse button (read)"),
        0x018 => n("SERDATR", "serial port data and status (read)"),
        0x01A => n("DSKBYTR", "disk data byte and status (read)"),
        0x01C => n("INTENAR", "interrupt enable bits (read)"),
        0x01E => n("INTREQR", "interrupt request bits (read)"),
        0x020 => n("DSKPTH", "disk DMA pointer, high word"),
        0x022 => n("DSKPTL", "disk DMA pointer, low word"),
        0x024 => n(
            "DSKLEN",
            "disk DMA length; writing it twice starts a transfer",
        ),
        0x026 => n("DSKDAT", "disk DMA data (write)"),
        0x028 => n("REFPTR", "refresh pointer"),
        0x02A => n("VPOSW", "beam vertical position (write)"),
        0x02C => n("VHPOSW", "beam position (write)"),
        0x02E => n("COPCON", "copper control: may the copper touch the blitter"),
        0x030 => n("SERDAT", "serial port data (write)"),
        0x032 => n("SERPER", "serial port speed"),
        0x034 => n("POTGO", "pot port control"),
        0x036 => n("JOYTEST", "write to all mouse/joystick counters"),
        0x038 => n("STREQU", "strobe: short frame vertical blank"),
        0x03A => n("STRVBL", "strobe: vertical blank"),
        0x03C => n("STRHOR", "strobe: horizontal sync"),
        0x03E => n("STRLONG", "strobe: long horizontal line"),
        0x040 => n("BLTCON0", "blitter control 0: channels and logic function"),
        0x042 => n("BLTCON1", "blitter control 1: shift, line and fill modes"),
        0x044 => n("BLTAFWM", "blitter first-word mask, source A"),
        0x046 => n("BLTALWM", "blitter last-word mask, source A"),
        0x048 => n("BLTCPTH", "blitter pointer, source C, high word"),
        0x04A => n("BLTCPTL", "blitter pointer, source C, low word"),
        0x04C => n("BLTBPTH", "blitter pointer, source B, high word"),
        0x04E => n("BLTBPTL", "blitter pointer, source B, low word"),
        0x050 => n("BLTAPTH", "blitter pointer, source A, high word"),
        0x052 => n("BLTAPTL", "blitter pointer, source A, low word"),
        0x054 => n("BLTDPTH", "blitter pointer, destination, high word"),
        0x056 => n("BLTDPTL", "blitter pointer, destination, low word"),
        0x058 => n("BLTSIZE", "blit size; writing it starts the blit"),
        0x05A => n("BLTCON0L", "blitter logic function only (ECS)"),
        0x05C => n("BLTSIZV", "blit height (ECS)"),
        0x05E => n("BLTSIZH", "blit width; writing it starts the blit (ECS)"),
        0x060 => n("BLTCMOD", "blitter modulo, source C"),
        0x062 => n("BLTBMOD", "blitter modulo, source B"),
        0x064 => n("BLTAMOD", "blitter modulo, source A"),
        0x066 => n("BLTDMOD", "blitter modulo, destination"),
        0x070 => n("BLTCDAT", "blitter data, source C"),
        0x072 => n("BLTBDAT", "blitter data, source B"),
        0x074 => n("BLTADAT", "blitter data, source A"),
        0x078 => n("SPRHDAT", "ext. logic UHRES sprite pointer and data (ECS)"),
        0x07C => n("DENISEID", "display chip revision (ECS/AGA, read)"),
        0x07E => n("DSKSYNC", "disk sync word the controller waits for"),
        0x080 => n("COP1LCH", "copper list 1 address, high word"),
        0x082 => n("COP1LCL", "copper list 1 address, low word"),
        0x084 => n("COP2LCH", "copper list 2 address, high word"),
        0x086 => n("COP2LCL", "copper list 2 address, low word"),
        0x088 => n("COPJMP1", "strobe: restart the copper at list 1"),
        0x08A => n("COPJMP2", "strobe: restart the copper at list 2"),
        0x08C => n("COPINS", "copper instruction fetch"),
        0x08E => n("DIWSTRT", "display window start"),
        0x090 => n("DIWSTOP", "display window stop"),
        0x092 => n("DDFSTRT", "display data fetch start"),
        0x094 => n("DDFSTOP", "display data fetch stop"),
        0x096 => n("DMACON", "DMA control: switch DMA channels on and off"),
        0x098 => n("CLXCON", "collision control"),
        0x09A => n("INTENA", "interrupt enable"),
        0x09C => n("INTREQ", "interrupt request: acknowledge or raise"),
        0x09E => n("ADKCON", "audio and disk control"),
        0x100 => n("BPLCON0", "bitplane control: number of planes, resolution"),
        0x102 => n("BPLCON1", "bitplane control: horizontal scroll"),
        0x104 => n("BPLCON2", "bitplane control: sprite/playfield priority"),
        0x106 => n("BPLCON3", "bitplane control 3 (ECS/AGA)"),
        0x108 => n("BPL1MOD", "bitplane modulo, odd planes"),
        0x10A => n("BPL2MOD", "bitplane modulo, even planes"),
        0x10C => n("BPLCON4", "bitplane control 4 (AGA)"),
        0x10E => n("CLXCON2", "collision control 2 (AGA)"),
        0x1C0 => n("HTOTAL", "horizontal line length (ECS)"),
        0x1C2 => n("HSSTOP", "horizontal sync stop (ECS)"),
        0x1C4 => n("HBSTRT", "horizontal blank start (ECS)"),
        0x1C6 => n("HBSTOP", "horizontal blank stop (ECS)"),
        0x1C8 => n("VTOTAL", "lines per frame (ECS)"),
        0x1CA => n("VSSTOP", "vertical sync stop (ECS)"),
        0x1CC => n("VBSTRT", "vertical blank start (ECS)"),
        0x1CE => n("VBSTOP", "vertical blank stop (ECS)"),
        0x1D0 => n("SPRHSTRT", "UHRES sprite vertical start (ECS)"),
        0x1D2 => n("SPRHSTOP", "UHRES sprite vertical stop (ECS)"),
        0x1D4 => n("BPLHSTRT", "UHRES bitplane vertical start (ECS)"),
        0x1D6 => n("BPLHSTOP", "UHRES bitplane vertical stop (ECS)"),
        0x1D8 => n("HHPOSW", "DUAL mode horizontal position (ECS, write)"),
        0x1DA => n("HHPOSR", "DUAL mode horizontal position (ECS, read)"),
        0x1DC => n("BEAMCON0", "beam control: PAL/NTSC and sync (ECS)"),
        0x1DE => n("HSSTRT", "horizontal sync start (ECS)"),
        0x1E0 => n("VSSTRT", "vertical sync start (ECS)"),
        0x1E2 => n("HCENTER", "horizontal position of vsync (ECS)"),
        0x1E4 => n("DIWHIGH", "display window high bits (ECS)"),
        0x1FC => n("FMODE", "memory fetch mode (AGA)"),
        _ => None,
    }
}

const BPLDAT: [&str; 8] = [
    "BPL1DAT", "BPL2DAT", "BPL3DAT", "BPL4DAT", "BPL5DAT", "BPL6DAT", "BPL7DAT", "BPL8DAT",
];

/// Which of a run of equally spaced registers `offset` is.
fn index(offset: u32, first: u32, stride: u32) -> Option<usize> {
    let rel = offset.checked_sub(first)?;
    if rel.checked_rem(stride)? != 0 {
        return None;
    }
    usize::try_from(rel.checked_div(stride)?).ok()
}

fn audio(offset: u32) -> Option<Named> {
    const NAMES: [[&str; 6]; 4] = [
        [
            "AUD0LCH", "AUD0LCL", "AUD0LEN", "AUD0PER", "AUD0VOL", "AUD0DAT",
        ],
        [
            "AUD1LCH", "AUD1LCL", "AUD1LEN", "AUD1PER", "AUD1VOL", "AUD1DAT",
        ],
        [
            "AUD2LCH", "AUD2LCL", "AUD2LEN", "AUD2PER", "AUD2VOL", "AUD2DAT",
        ],
        [
            "AUD3LCH", "AUD3LCL", "AUD3LEN", "AUD3PER", "AUD3VOL", "AUD3DAT",
        ],
    ];
    const MEANINGS: [&str; 6] = [
        "audio sample address, high word",
        "audio sample address, low word",
        "audio sample length in words",
        "audio period (pitch)",
        "audio volume, 0-64",
        "audio data",
    ];
    let rel = offset.checked_sub(0xA0)?;
    let channel = usize::try_from(rel / 0x10).ok()?;
    let reg = usize::try_from((rel % 0x10) / 2).ok()?;
    Some(Named {
        name: NAMES.get(channel)?.get(reg)?,
        meaning: MEANINGS.get(reg)?,
    })
}

fn bitplane_pointer(offset: u32) -> Option<Named> {
    const NAMES: [&str; 16] = [
        "BPL1PTH", "BPL1PTL", "BPL2PTH", "BPL2PTL", "BPL3PTH", "BPL3PTL", "BPL4PTH", "BPL4PTL",
        "BPL5PTH", "BPL5PTL", "BPL6PTH", "BPL6PTL", "BPL7PTH", "BPL7PTL", "BPL8PTH", "BPL8PTL",
    ];
    let i = index(offset, 0xE0, 2)?;
    Some(Named {
        name: NAMES.get(i)?,
        meaning: if i % 2 == 0 {
            "bitplane address, high word"
        } else {
            "bitplane address, low word"
        },
    })
}

fn sprite_pointer(offset: u32) -> Option<Named> {
    const NAMES: [&str; 16] = [
        "SPR0PTH", "SPR0PTL", "SPR1PTH", "SPR1PTL", "SPR2PTH", "SPR2PTL", "SPR3PTH", "SPR3PTL",
        "SPR4PTH", "SPR4PTL", "SPR5PTH", "SPR5PTL", "SPR6PTH", "SPR6PTL", "SPR7PTH", "SPR7PTL",
    ];
    let i = index(offset, 0x120, 2)?;
    Some(Named {
        name: NAMES.get(i)?,
        meaning: if i % 2 == 0 {
            "sprite data address, high word"
        } else {
            "sprite data address, low word"
        },
    })
}

fn sprite_control(offset: u32) -> Option<Named> {
    const NAMES: [[&str; 4]; 8] = [
        ["SPR0POS", "SPR0CTL", "SPR0DATA", "SPR0DATB"],
        ["SPR1POS", "SPR1CTL", "SPR1DATA", "SPR1DATB"],
        ["SPR2POS", "SPR2CTL", "SPR2DATA", "SPR2DATB"],
        ["SPR3POS", "SPR3CTL", "SPR3DATA", "SPR3DATB"],
        ["SPR4POS", "SPR4CTL", "SPR4DATA", "SPR4DATB"],
        ["SPR5POS", "SPR5CTL", "SPR5DATA", "SPR5DATB"],
        ["SPR6POS", "SPR6CTL", "SPR6DATA", "SPR6DATB"],
        ["SPR7POS", "SPR7CTL", "SPR7DATA", "SPR7DATB"],
    ];
    const MEANINGS: [&str; 4] = [
        "sprite start position",
        "sprite stop position and control",
        "sprite image data, plane A",
        "sprite image data, plane B",
    ];
    let rel = offset.checked_sub(0x140)?;
    let sprite = usize::try_from(rel / 8).ok()?;
    let reg = usize::try_from((rel % 8) / 2).ok()?;
    Some(Named {
        name: NAMES.get(sprite)?.get(reg)?,
        meaning: MEANINGS.get(reg)?,
    })
}

fn colour(offset: u32) -> Option<Named> {
    const NAMES: [&str; 32] = [
        "COLOR00", "COLOR01", "COLOR02", "COLOR03", "COLOR04", "COLOR05", "COLOR06", "COLOR07",
        "COLOR08", "COLOR09", "COLOR10", "COLOR11", "COLOR12", "COLOR13", "COLOR14", "COLOR15",
        "COLOR16", "COLOR17", "COLOR18", "COLOR19", "COLOR20", "COLOR21", "COLOR22", "COLOR23",
        "COLOR24", "COLOR25", "COLOR26", "COLOR27", "COLOR28", "COLOR29", "COLOR30", "COLOR31",
    ];
    let i = index(offset, 0x180, 2)?;
    Some(Named {
        name: NAMES.get(i)?,
        meaning: if i == 0 {
            "colour register 0: the background"
        } else {
            "colour register"
        },
    })
}

/// A register of one of the two 8520 CIAs.
///
/// CIA-A answers on odd addresses from `$bfe001` and CIA-B on even ones from
/// `$bfd000`, one register every `$100`.
fn cia(address: u32) -> Option<Named> {
    const A: [(&str, &str); 16] = [
        (
            "CIAA PRA",
            "CIA-A port A: fire buttons, disk status, LED, ROM overlay",
        ),
        ("CIAA PRB", "CIA-A port B: parallel port data"),
        ("CIAA DDRA", "CIA-A port A direction"),
        ("CIAA DDRB", "CIA-A port B direction"),
        ("CIAA TALO", "CIA-A timer A, low byte"),
        ("CIAA TAHI", "CIA-A timer A, high byte"),
        ("CIAA TBLO", "CIA-A timer B, low byte"),
        ("CIAA TBHI", "CIA-A timer B, high byte"),
        (
            "CIAA TODLO",
            "CIA-A event counter (vertical blanks), low byte",
        ),
        ("CIAA TODMID", "CIA-A event counter, middle byte"),
        ("CIAA TODHI", "CIA-A event counter, high byte"),
        ("CIAA", "CIA-A unused register"),
        ("CIAA SDR", "CIA-A serial data: the keyboard"),
        ("CIAA ICR", "CIA-A interrupt control"),
        ("CIAA CRA", "CIA-A control register A"),
        ("CIAA CRB", "CIA-A control register B"),
    ];
    const B: [(&str, &str); 16] = [
        (
            "CIAB PRA",
            "CIA-B port A: serial and parallel control lines",
        ),
        (
            "CIAB PRB",
            "CIA-B port B: floppy drive control (motor, select, side, step)",
        ),
        ("CIAB DDRA", "CIA-B port A direction"),
        ("CIAB DDRB", "CIA-B port B direction"),
        ("CIAB TALO", "CIA-B timer A, low byte"),
        ("CIAB TAHI", "CIA-B timer A, high byte"),
        ("CIAB TBLO", "CIA-B timer B, low byte"),
        ("CIAB TBHI", "CIA-B timer B, high byte"),
        (
            "CIAB TODLO",
            "CIA-B event counter (horizontal syncs), low byte",
        ),
        ("CIAB TODMID", "CIA-B event counter, middle byte"),
        ("CIAB TODHI", "CIA-B event counter, high byte"),
        ("CIAB", "CIA-B unused register"),
        ("CIAB SDR", "CIA-B serial data"),
        ("CIAB ICR", "CIA-B interrupt control"),
        ("CIAB CRA", "CIA-B control register A"),
        ("CIAB CRB", "CIA-B control register B"),
    ];
    let (table, base) = match address {
        // One register every $100: CIA-A on the odd byte, CIA-B on the even.
        0x00BF_E001..=0x00BF_EF01 if address.wrapping_sub(1).trailing_zeros() >= 8 => {
            (&A, 0x00BF_E001)
        }
        0x00BF_D000..=0x00BF_DF00 if address.trailing_zeros() >= 8 => (&B, 0x00BF_D000),
        _ => return None,
    };
    let reg = usize::try_from(address.checked_sub(base)? >> 8).ok()?;
    let (name, meaning) = table.get(reg)?;
    Some(Named { name, meaning })
}

/// What a bit of a CIA port means, for the two ports programs test bit by bit.
///
/// `btst #6,$bfe001` is how a game waits for the left mouse button, and
/// `bclr #3,$bfd100` selects drive 0. The `/` marks an active-low signal: the
/// bit is 0 when the thing is true.
#[must_use]
pub fn port_bit(address: u32, bit: u32) -> Option<&'static str> {
    const CIAA_PRA: [&str; 8] = [
        "OVL: ROM overlay at address 0",
        "LED: power light dim/filter off",
        "/CHNG: disk changed",
        "/WPRO: disk write-protected",
        "/TK0: drive head at track 0",
        "/RDY: drive ready",
        "/FIR0: port 1 fire button (left mouse button)",
        "/FIR1: port 2 fire button (joystick fire)",
    ];
    const CIAB_PRB: [&str; 8] = [
        "/STEP: step the drive heads",
        "/DIR: step direction (0 = inwards)",
        "/SIDE: disk side (0 = upper head)",
        "/SEL0: select drive 0 (internal)",
        "/SEL1: select drive 1",
        "/SEL2: select drive 2",
        "/SEL3: select drive 3",
        "/MTR: drive motor (0 = on, latched at select)",
    ];
    let table = match address {
        0x00BF_E001 => &CIAA_PRA,
        0x00BF_D100 => &CIAB_PRB,
        _ => return None,
    };
    table.get(usize::try_from(bit).ok()?).copied()
}

/// The 68000's exception vectors and the ExecBase pointer, in the first
/// kilobyte of memory.
fn vector(address: u32) -> Option<Named> {
    let n = |name, meaning| Some(Named { name, meaning });
    match address {
        0x04 => n(
            "ExecBase",
            "pointer to exec.library, the one fixed address in AmigaOS",
        ),
        0x08 => n("bus error vector", "68000 exception vector"),
        0x0C => n("address error vector", "68000 exception vector"),
        0x10 => n("illegal instruction vector", "68000 exception vector"),
        0x14 => n("divide by zero vector", "68000 exception vector"),
        0x18 => n("CHK vector", "68000 exception vector"),
        0x1C => n("TRAPV vector", "68000 exception vector"),
        0x20 => n("privilege violation vector", "68000 exception vector"),
        0x24 => n("trace vector", "68000 exception vector"),
        0x28 => n("line-A vector", "68000 exception vector"),
        0x2C => n("line-F vector", "68000 exception vector"),
        0x60 => n("spurious interrupt vector", "68000 exception vector"),
        0x64 => n(
            "level 1 interrupt vector",
            "serial out, disk block done, software",
        ),
        0x68 => n("level 2 interrupt vector", "CIA-A: keyboard, timers"),
        0x6C => n(
            "level 3 interrupt vector",
            "vertical blank, copper, blitter",
        ),
        0x70 => n("level 4 interrupt vector", "audio channels"),
        0x74 => n("level 5 interrupt vector", "serial in, disk sync"),
        0x78 => n("level 6 interrupt vector", "CIA-B: timers, external"),
        0x7C => n(
            "level 7 interrupt vector",
            "non-maskable (unused on an Amiga)",
        ),
        0x80..=0xBC if address % 4 == 0 => trap_vector(address),
        _ => None,
    }
}

fn trap_vector(address: u32) -> Option<Named> {
    const NAMES: [&str; 16] = [
        "TRAP #0 vector",
        "TRAP #1 vector",
        "TRAP #2 vector",
        "TRAP #3 vector",
        "TRAP #4 vector",
        "TRAP #5 vector",
        "TRAP #6 vector",
        "TRAP #7 vector",
        "TRAP #8 vector",
        "TRAP #9 vector",
        "TRAP #10 vector",
        "TRAP #11 vector",
        "TRAP #12 vector",
        "TRAP #13 vector",
        "TRAP #14 vector",
        "TRAP #15 vector",
    ];
    let i = index(address, 0x80, 4)?;
    Some(Named {
        name: NAMES.get(i)?,
        meaning: "68000 exception vector",
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn the_addresses_every_amiga_program_uses_are_named() {
        assert_eq!(name(0x00DF_F096).unwrap().name, "DMACON");
        assert_eq!(name(0x00DF_F09A).unwrap().name, "INTENA");
        assert_eq!(name(0x00DF_F180).unwrap().name, "COLOR00");
        assert_eq!(name(0x00DF_F1BE).unwrap().name, "COLOR31");
        assert_eq!(name(0x00DF_F080).unwrap().name, "COP1LCH");
        assert_eq!(name(0x00DF_F0A8).unwrap().name, "AUD0VOL");
        assert_eq!(name(0x00DF_F0D8).unwrap().name, "AUD3VOL");
        assert_eq!(name(0x00DF_F0E0).unwrap().name, "BPL1PTH");
        assert_eq!(name(0x00DF_F0FE).unwrap().name, "BPL8PTL");
        assert_eq!(name(0x00DF_F140).unwrap().name, "SPR0POS");
        assert_eq!(name(0x00DF_F17E).unwrap().name, "SPR7DATB");
        assert_eq!(name(0x00BF_E001).unwrap().name, "CIAA PRA");
        assert_eq!(name(0x00BF_EC01).unwrap().name, "CIAA SDR");
        assert_eq!(name(0x00BF_D100).unwrap().name, "CIAB PRB");
        assert_eq!(name(0x4).unwrap().name, "ExecBase");
        assert_eq!(name(0x6C).unwrap().name, "level 3 interrupt vector");
        assert_eq!(name(0xBC).unwrap().name, "TRAP #15 vector");
        assert_eq!(
            port_bit(0x00BF_E001, 6),
            Some("/FIR0: port 1 fire button (left mouse button)")
        );
        assert_eq!(
            port_bit(0x00BF_D100, 7),
            Some("/MTR: drive motor (0 = on, latched at select)")
        );
    }

    #[test]
    fn ordinary_memory_and_the_wrong_cia_half_are_not_named() {
        // Chip memory is whatever a program puts there.
        assert_eq!(name(0x1000), None);
        assert_eq!(name(0x400), None);
        // CIA-A is on odd addresses, CIA-B on even ones.
        assert_eq!(name(0x00BF_E000), None);
        assert_eq!(name(0x00BF_D101), None);
        // Past the last custom register, and between registers.
        assert_eq!(name(0x00DF_F200), None);
        assert_eq!(name(0x00DF_F097), None);
        assert_eq!(
            name(0x00DF_F0A1),
            None,
            "odd offsets are not audio registers"
        );
        assert_eq!(port_bit(0x00BF_E001, 8), None);
    }
}
