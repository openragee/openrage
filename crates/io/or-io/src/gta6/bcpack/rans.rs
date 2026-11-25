// bcpack-rs by Ahmet Dedeler (https://github.com/Ahmet-Dedeler, https://ahmetdedeler.com)
// Source: https://github.com/Ahmet-Dedeler/bcpack-rs
// SPDX-License-Identifier: MIT OR Apache-2.0
// If you copy, vendor or port this file, keep this header and credit bcpack-rs in your README.
//
// Origin: bcpack-rs, `src/rans.rs`, commit 5bf5cc56ed56f1ababf1f9592300bd182ff90bd8, taken under
// its MIT licence: its clocked decoder alone, reading through a closure and growing each lane's
// symbols as they arrive. Each lane is, upstream, a port of the `decoder` module of
// mdu_minirans.v in https://github.com/microsoft/silicon-rans, MIT License, Copyright (c)
// Microsoft Corporation. The notices are in LICENSE-third-party.md.
//! The entropy layer: several rANS lanes that share one byte stream, a clock at a time.
//!
//! Which lane takes which byte depends on every lane's state, so the lanes are stepped together:
//! each clock serves at most [`FEED_WINDOW`] bytes in lane order, and a lane hands over a symbol
//! only once every lane still running holds one.

use super::tables::{BASE_TABLE, FREQ_TABLE, RF_TABLE, RN_TABLE, RS_TABLE};
use super::BcPackError;

/// The state a lane renormalises up to, and returns to at the end of a fragment.
pub(super) const L: u32 = 1 << 24;
/// The bytes the stream hands to the lanes in one clock.
const FEED_WINDOW: usize = 8;
/// Clocks with no byte read, no symbol handed over and no state change before the stream is given up on.
const STALLED_CLOCKS: u64 = 64;

/// One static model: symbols of `eab + 1` bits, in segments of one frequency each (segment 0 is
/// symbol 0, segment k is the symbols from 2^(k-1) up to 2^k).
#[derive(Debug, Clone, Copy)]
pub(super) struct Model {
    base: [u16; 10],
    freq: [u16; 10],
    rf: [u32; 10],
    rs: [u8; 10],
    rn: [u16; 10],
    segments: usize,
}

impl Model {
    /// The model of alphabet index `eab` (0 to 8) and number `q` (0 to 15).
    pub(super) fn of(eab: usize, q: usize) -> Self {
        Self {
            base: BASE_TABLE[eab][q],
            freq: FREQ_TABLE[eab][q],
            rf: RF_TABLE[eab][q],
            rs: RS_TABLE[eab][q],
            rn: RN_TABLE[eab][q],
            segments: eab + 2,
        }
    }

    /// The symbol whose interval holds the state's low 16 bits: the last segment starting at or
    /// below them, then a multiply by the reciprocal of the segment's frequency.
    pub(super) fn inverse(&self, cf: u16) -> u16 {
        let cf = u32::from(cf);
        let segment = (1..self.segments)
            .rev()
            .find(|&k| cf >= u32::from(self.base[k]))
            .unwrap_or(0);
        let first = if segment == 0 {
            0
        } else {
            1u32 << (segment - 1)
        };
        let distance = cf
            .wrapping_sub(u32::from(self.base[segment]))
            .wrapping_add(u32::from(self.rn[segment]))
            & 0xFFFF;
        let steps = ((distance.wrapping_mul(self.rf[segment]) >> 15) >> self.rs[segment]) & 0x1FF;
        ((first + steps) & 0x1FF) as u16
    }

    /// The frequency and cumulative frequency of `symbol`. Cumulative frequency 0 is reserved, so
    /// the symbol that would start there starts at 1 with one less.
    pub(super) fn forward(&self, symbol: u16) -> (u32, u32) {
        let symbol = u32::from(symbol);
        let segment = (32 - symbol.leading_zeros()) as usize;
        let first = if segment == 0 {
            0
        } else {
            1u32 << (segment - 1)
        };
        let frequency = u32::from(self.freq[segment]);
        let start = u32::from(self.base[segment])
            .wrapping_add(frequency.wrapping_mul(symbol - first))
            & 0xFFFF;
        if start == 0 {
            (frequency.wrapping_sub(1) & 0xFFFF, 1)
        } else {
            (frequency, start)
        }
    }

    /// The state after `symbol` is decoded from state `x`.
    fn advance(&self, x: u32, symbol: u16) -> u32 {
        let (frequency, start) = self.forward(symbol);
        frequency
            .wrapping_mul(x >> 16)
            .wrapping_add(x & 0xFFFF)
            .wrapping_sub(start)
    }
}

/// Where a lane is in its fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Idle,
    Header0,
    Header1,
    Header2,
    /// Reading the fragment's bytes.
    Reading,
    /// Its bytes read, decoding until the state is back at [`L`].
    Draining,
}

struct Lane {
    /// The lane's symbol width less one.
    alphabet_bits: u8,
    target: u32,
    step: Step,
    /// The one byte the lane holds unread.
    byte: u8,
    has_byte: bool,
    /// The one symbol the lane holds until every lane has one.
    symbol: u16,
    has_symbol: bool,
    /// Bytes the fragment still gives the lane.
    bytes_left: u32,
    symbols_left: u32,
    /// The state at each of a clock's two phases.
    phase0: u32,
    phase1: u32,
    long_length: bool,
    flush: bool,
    q: u8,
    eab: u8,
    model: Model,
    started: bool,
    done: bool,
    stalled: bool,
    end_state: u32,
}

impl Lane {
    fn new(alphabet_bits: u8, target: u32) -> Self {
        Self {
            alphabet_bits,
            target,
            step: Step::Idle,
            byte: 0,
            has_byte: false,
            symbol: 0,
            has_symbol: false,
            bytes_left: 0,
            symbols_left: 0,
            phase0: 0,
            phase1: 0,
            long_length: false,
            flush: false,
            q: 0,
            eab: 0,
            model: Model::of(0, 0),
            started: false,
            done: false,
            stalled: false,
            end_state: 0,
        }
    }

    fn wants_byte(&self) -> bool {
        !self.has_byte
            && match self.step {
                Step::Header0 | Step::Header1 | Step::Header2 => true,
                Step::Reading => self.bytes_left > 0,
                Step::Idle | Step::Draining => false,
            }
    }

    /// The second phase of a clock: the fragment header, renormalisation and one symbol.
    fn second_phase(&mut self, lane: usize) -> Result<(), BcPackError> {
        match self.step {
            Step::Idle => {
                if !self.started {
                    self.started = true;
                    self.step = Step::Header0;
                    self.symbols_left = self.target;
                    (self.phase0, self.phase1) = (0, 0);
                }
            }
            Step::Header0 => {
                if self.has_byte {
                    let byte = self.byte;
                    self.has_byte = false;
                    self.bytes_left = 0;
                    self.step = Step::Header1;
                    self.q = byte & 15;
                    self.long_length = byte & 0x20 != 0;
                    self.flush = byte & 0x10 != 0;
                    // The top two bits narrow the lane's alphabet.
                    let eab = (i32::from(self.alphabet_bits) - i32::from(byte >> 6)) & 15;
                    if eab > 8 {
                        return Err(BcPackError::Header { lane });
                    }
                    self.eab = eab as u8;
                }
            }
            Step::Header1 => {
                if self.has_byte {
                    self.has_byte = false;
                    self.model = Model::of(self.eab.into(), self.q.into());
                    if self.long_length {
                        self.bytes_left = self.byte.into();
                        self.step = Step::Header2;
                    } else {
                        self.bytes_left = u32::from(self.byte) + 1;
                        self.step = Step::Reading;
                    }
                }
            }
            Step::Header2 => {
                if self.has_byte {
                    self.has_byte = false;
                    self.bytes_left = (u32::from(self.byte) << 8 | (self.bytes_left & 0xFF)) + 257;
                    self.step = Step::Reading;
                }
            }
            Step::Reading | Step::Draining => {
                let reading = self.step == Step::Reading;
                let takes = reading && self.phase0 < L && self.bytes_left > 0 && self.has_byte;
                let x = if takes {
                    (self.phase0 & 0xFF_FFFF) << 8 | u32::from(self.byte)
                } else {
                    self.phase0
                };
                let decodes = x > L && self.symbols_left > 0;
                if decodes {
                    self.symbols_left -= 1;
                    self.symbol = self.model.inverse(x as u16);
                }
                self.phase1 = x;
                self.has_symbol = decodes;
                if takes {
                    self.bytes_left -= 1;
                    self.has_byte = false;
                }
                let fragment_over = if reading {
                    self.bytes_left == 0
                } else {
                    x <= L
                };
                if !fragment_over {
                    return Ok(());
                }
                if self.symbols_left == 0 {
                    self.done = true;
                    self.step = Step::Idle;
                    self.end_state = if decodes {
                        self.model.advance(x, self.symbol)
                    } else {
                        x
                    };
                } else if reading && x != L {
                    self.step = Step::Draining;
                } else {
                    self.step = Step::Header0;
                    if self.flush {
                        self.phase1 = 0;
                        if !reading {
                            self.phase0 = 0;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// `target` symbols of each lane, and the bytes read, from the stream `next` hands out by lane.
///
/// `alphabet_bits` holds each lane's symbol width less one, and `stream_len` bounds the clocks.
pub(super) fn decode_lanes(
    next: impl FnMut(usize) -> Option<u8>,
    alphabet_bits: &[u8],
    target: usize,
    stream_len: usize,
) -> Result<(Vec<Vec<u16>>, usize), BcPackError> {
    decode_lanes_until(next, alphabet_bits, target, stream_len, target)
}

/// [`decode_lanes`], stopping with the stream unfinished once every lane holds `prefix` symbols.
pub(super) fn decode_lanes_until(
    mut next: impl FnMut(usize) -> Option<u8>,
    alphabet_bits: &[u8],
    target: usize,
    stream_len: usize,
    prefix: usize,
) -> Result<(Vec<Vec<u16>>, usize), BcPackError> {
    let count = alphabet_bits.len();
    let symbols_each = u32::try_from(target)
        .ok()
        .filter(|&n| n > 0 && count > 0 && count <= 32)
        .ok_or(BcPackError::Size {
            size: 8 * target as u64,
        })?;
    let mut lanes: Vec<Lane> = alphabet_bits
        .iter()
        .map(|&bits| Lane::new(bits, symbols_each))
        .collect();
    let mut symbols: Vec<Vec<u16>> = vec![Vec::new(); count];
    // The lanes whose handed-over symbol waits for the others.
    let mut slots = 0u32;
    let (mut consumed, mut exhausted, mut handed) = (0usize, false, 0u64);
    let clocks = 64 * (count as u64 * target as u64 + stream_len as u64) + 1000;
    let mut progress = (usize::MAX, u64::MAX, u32::MAX);
    let mut changed_at = 0u64;

    for clock in 0..clocks {
        let mut served = 0;
        for (i, lane) in lanes.iter_mut().enumerate() {
            let bit = 1u32 << i;
            let blocked = lane.has_symbol && slots & bit != 0;
            let wants = lane.wants_byte();
            let mut byte = None;
            if wants && !blocked && served < FEED_WINDOW && !exhausted {
                byte = next(i);
                exhausted = byte.is_none();
            }
            lane.stalled = (wants && byte.is_none()) || blocked;
            if lane.stalled {
                continue;
            }
            if matches!(lane.step, Step::Reading | Step::Draining) {
                lane.phase0 = if lane.has_symbol {
                    lane.model.advance(lane.phase1, lane.symbol)
                } else {
                    lane.phase1
                };
            }
            if let Some(byte) = byte {
                (lane.byte, lane.has_byte) = (byte, true);
                consumed += 1;
                served += 1;
            }
            if lane.has_symbol {
                slots |= bit;
                symbols[i].push(lane.symbol);
                handed += 1;
                lane.has_symbol = false;
            }
        }

        let (mut running, mut finished, mut steps) = (0u32, true, 0u32);
        for (i, lane) in lanes.iter_mut().enumerate() {
            if !lane.stalled {
                lane.second_phase(i)?;
            }
            let bit = 1u32 << i;
            if !(lane.done && !lane.has_symbol && slots & bit == 0) {
                running |= bit;
            }
            finished &= lane.done && !lane.has_symbol;
            steps += lane.step as u32;
        }
        if slots != 0 && slots & running == running {
            slots = 0;
        }
        if prefix < target && symbols.iter().all(|lane| lane.len() >= prefix) {
            return Ok((symbols, consumed));
        }
        if finished {
            // A coder may leave off up to two zero bytes at the end of a lane.
            let lanes: Vec<usize> = (0..count)
                .filter(|&i| ![L, L >> 8, L >> 16].contains(&lanes[i].end_state))
                .collect();
            if !lanes.is_empty() {
                return Err(BcPackError::EndState { lanes });
            }
            return Ok((symbols, consumed));
        }
        let now = (consumed, handed, steps);
        if now != progress {
            (progress, changed_at) = (now, clock);
        } else if clock - changed_at > STALLED_CLOCKS {
            break;
        }
    }
    Err(BcPackError::Truncated { consumed })
}
