//! The checksum the GTA VI PSO loader recomputes over a `CHKS`-framed body and rejects on
//! mismatch: a seeded, signed-byte Jenkins one-at-a-time with the standard finalisation. The
//! 0x14-byte frame starts from a fixed seed; the tagless 0x10-byte frame starts from zero. The
//! 0x14 form is reproduced on every clear (unciphered) PSO body a build holds; no 0x10-framed body
//! occurs in a build to census, so its rule stands as disassembled and a save leaves a 0x10 frame
//! carried rather than writing an unverified value.

/// The seed the loader starts from for the 0x14-byte `CHKS` frame; the 0x10-byte frame starts
/// from zero.
const SEED_0X14: u32 = 0x3FAC_7125;
/// The multiplier each step applies after adding the next byte: `h = (h + c) * MIX`.
const MIX: u32 = 0x401;
/// The finalisation multipliers: `((h * 9) ^ ((h * 9) >> 11)) * SCATTER`.
const SCATTER: u32 = 0x8001;

/// The checksum of a `CHKS`-framed body, or `None` when it carries no trailing frame. The body's
/// length and checksum words (the frame's `+8` and `+12`) are taken as zero, as the loader does,
/// and every byte is sign-extended before it is added.
pub(crate) fn checksum(body: &[u8]) -> Option<u32> {
    let (frame, seed) = frame(body)?;
    let total = u32::from_be_bytes(body.get(frame + 8..frame + 12)?.try_into().ok()?) as usize;
    let end = total.max(frame + 20).min(body.len());
    let mut h = seed;
    for (i, &byte) in body[..end].iter().enumerate() {
        let c = if (frame + 8..frame + 16).contains(&i) {
            0
        } else {
            byte as i8 as i32 as u32
        };
        h = h.wrapping_add(c).wrapping_mul(MIX);
        h ^= h >> 6;
    }
    let h9 = h.wrapping_mul(9);
    Some((h9 ^ (h9 >> 11)).wrapping_mul(SCATTER))
}

/// Writes the recomputed checksum into a GTA VI `zppp`-tagged 0x14-byte `CHKS` frame; a no-op for
/// any other body. A GTA V/RDR2 `yppp` frame uses a different, unmeasured rule, and a tagless 0x10
/// frame's rule is not verified against any file (none occur in a build), so both are left carried
/// as they were read — the bodies [`super::keeps_stale_checksum`] reports.
pub(crate) fn write_checksum(body: &mut [u8]) {
    let Some((frame, _)) = frame(body) else {
        return;
    };
    // Only the `zppp`-tagged 0x14 frame is recomputed; its tag sits at `frame + 16..frame + 20`.
    // A 0x10 frame has no room there, so `get` yields `None` and it is left carried.
    if body.get(frame + 16..frame + 20) != Some(b"zppp".as_slice()) {
        return;
    }
    if let Some(sum) = checksum(body) {
        body[frame + 12..frame + 16].copy_from_slice(&sum.to_be_bytes());
    }
}

/// The start of the trailing `CHKS` frame and the seed its length word selects: the 0x14-byte
/// frame `{"CHKS", BE 0x14, BE length, BE checksum, tag}` or the tagless 0x10-byte frame.
fn frame(body: &[u8]) -> Option<(usize, u32)> {
    let len = body.len();
    if len >= 20
        && &body[len - 20..len - 16] == b"CHKS"
        && body[len - 16..len - 12] == 0x14u32.to_be_bytes()
    {
        return Some((len - 20, SEED_0X14));
    }
    if len >= 16
        && &body[len - 16..len - 12] == b"CHKS"
        && body[len - 12..len - 8] == 0x10u32.to_be_bytes()
    {
        return Some((len - 16, 0));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-built GTA VI 0x14-frame body: 28 bytes of arbitrary content (not game bytes), then
    /// the frame `{"CHKS", BE 0x14, BE 48, BE checksum, "zppp"}`.
    fn body_0x14(stored: u32) -> Vec<u8> {
        let mut out: Vec<u8> = (0u8..28).collect();
        out.extend_from_slice(b"CHKS");
        out.extend_from_slice(&0x14u32.to_be_bytes());
        out.extend_from_slice(&48u32.to_be_bytes());
        out.extend_from_slice(&stored.to_be_bytes());
        out.extend_from_slice(b"zppp");
        assert_eq!(out.len(), 48);
        out
    }

    /// A hand-built tagless 0x10-frame body: 16 bytes of content, then `{"CHKS", BE 0x10, BE 32,
    /// BE checksum}`.
    fn body_0x10(stored: u32) -> Vec<u8> {
        let mut out: Vec<u8> = (0x80u8..0x90).collect();
        out.extend_from_slice(b"CHKS");
        out.extend_from_slice(&0x10u32.to_be_bytes());
        out.extend_from_slice(&32u32.to_be_bytes());
        out.extend_from_slice(&stored.to_be_bytes());
        assert_eq!(out.len(), 32);
        out
    }

    #[test]
    fn the_rule_reproduces_the_loaders_value() {
        // The 0x14 vector is the rule reproduced across every clear PSO body of a build. The 0x10
        // vector only pins the same rule applied to a hand-built body under the zero seed: no 0x10
        // body occurs in a build to check it against, so a save leaves a 0x10 frame carried.
        assert_eq!(checksum(&body_0x14(0)), Some(0x0B35_7D62));
        assert_eq!(checksum(&body_0x10(0)), Some(0xA7D1_926B));
    }

    #[test]
    fn the_stored_word_does_not_feed_the_checksum() {
        // The checksum does not depend on the length or checksum words, which the rule zeroes.
        assert_eq!(checksum(&body_0x14(0)), checksum(&body_0x14(0xDEAD_BEEF)));
    }

    #[test]
    fn write_checksum_makes_the_frame_satisfy_the_rule() {
        let mut body = body_0x14(0xDEAD_BEEF);
        assert_ne!(
            checksum(&body),
            Some(0xDEAD_BEEF),
            "a deliberately stale value fails the rule"
        );
        write_checksum(&mut body);
        let frame = body.len() - 20;
        let stored = u32::from_be_bytes(body[frame + 12..frame + 16].try_into().unwrap());
        assert_eq!(
            checksum(&body),
            Some(stored),
            "the written value satisfies the rule"
        );
        assert_ne!(stored, 0xDEAD_BEEF, "an edit replaces the carried value");
    }

    #[test]
    fn a_tagless_0x10_frame_is_left_carried() {
        // A 0x10 frame's rule is not verified against any file, so a save leaves its checksum as it
        // was read rather than writing the disassembled value.
        let mut body = body_0x10(0xDEAD_BEEF);
        let before = body.clone();
        write_checksum(&mut body);
        assert_eq!(body, before);
    }

    #[test]
    fn keeps_stale_checksum_reports_which_frames_a_save_carries() {
        use super::super::keeps_stale_checksum;
        // A GTA VI `zppp` 0x14 frame is recomputed, so it is not carried.
        assert!(!keeps_stale_checksum(&body_0x14(0)));
        // A GTA V/RDR2 `yppp` 0x14 frame and a tagless 0x10 frame are carried.
        let mut yppp = body_0x14(0);
        yppp[44..48].copy_from_slice(b"yppp");
        assert!(keeps_stale_checksum(&yppp));
        assert!(keeps_stale_checksum(&body_0x10(0)));
        // A body with no frame carries nothing.
        assert!(!keeps_stale_checksum(b"PSIN not a framed body at all!!!"));
    }

    #[test]
    fn a_gta5_rdr2_tagged_frame_is_left_untouched() {
        // A `yppp`-tagged 0x14 frame uses a different, unmeasured rule and must not be rewritten.
        let mut body = body_0x14(0xDEAD_BEEF);
        body[44..48].copy_from_slice(b"yppp");
        let before = body.clone();
        write_checksum(&mut body);
        assert_eq!(body, before);
    }

    #[test]
    fn a_body_with_no_frame_is_left_alone() {
        let mut body = b"PSIN not a framed body at all!!!".to_vec();
        assert_eq!(checksum(&body), None);
        let before = body.clone();
        write_checksum(&mut body);
        assert_eq!(body, before);
    }

    /// Install-gated census: every unedited clear PSO body the top-level archives of a build hold
    /// carries a `CHKS` the rule reproduces exactly, counted per frame form. Skips cleanly without
    /// the key snapshot and the build, so a clean checkout passes. Reads only clear (unciphered)
    /// leaf bodies, which need no content key; nested archives are not walked.
    #[cfg(feature = "gta6")]
    #[test]
    fn every_clear_pso_body_in_a_build_satisfies_the_rule() {
        use std::fs::File;
        use std::path::PathBuf;

        use crate::container::rpf::{Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV};

        const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
        // The inflate stays cheap: a body larger than this is skipped, and enough small PSO bodies
        // remain to census the rule.
        const CAP: u64 = 0x40_0000;

        let what = "GTA VI PSO CHKS census over a build";
        let env_path = |var: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.exists())
        };
        let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV))
        else {
            or_test_support::skip_install(
                what,
                format_args!("set {GTA6_SNAPSHOT_ENV} and {BUILD_ENV}"),
            );
            return;
        };
        let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");

        let mut archives: Vec<PathBuf> = std::fs::read_dir(&build)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rpf"))
            .collect();
        archives.sort();
        assert!(!archives.is_empty(), "no .rpf under {}", build.display());

        // Verified counts kept per frame form, so the census reports which form it actually covered
        // rather than one total that a `zppp` body alone satisfies.
        let (mut verified_14, mut verified_10) = (0usize, 0usize);
        let mut mismatches: Vec<String> = Vec::new();
        for path in &archives {
            let mut file = File::open(path).expect("open the archive");
            let Ok(mount) = Gta6Mount::open_root(&mut file, &keys) else {
                continue;
            };
            for entry in &mount.archive.entries {
                if entry.cipher.is_some() || entry.stored.is_none_or(|s| s == 0 || s > CAP) {
                    continue;
                }
                let Ok(body) = mount.read_entry(&mut file, entry) else {
                    continue;
                };
                if !body.starts_with(b"PSIN") {
                    continue;
                }
                let Some((frame, _)) = frame(&body) else {
                    continue;
                };
                // Only GTA VI's own frames: a `zppp` 0x14 tag or the tagless 0x10 form.
                let zppp = body.get(frame + 16..frame + 20) == Some(b"zppp".as_slice());
                let tagless_10 = frame == body.len() - 16;
                if !zppp && !tagless_10 {
                    continue;
                }
                let stored = u32::from_be_bytes(body[frame + 12..frame + 16].try_into().unwrap());
                let computed = checksum(&body).expect("a framed body has a checksum");
                if stored == computed {
                    if zppp {
                        verified_14 += 1;
                    } else {
                        verified_10 += 1;
                    }
                } else if mismatches.len() < 8 {
                    mismatches.push(format!(
                        "{} entry {}: stored {stored:#010X} != computed {computed:#010X}",
                        path.display(),
                        entry.index
                    ));
                }
            }
        }

        println!(
            "CHKS census (top-level archives): {verified_14} zppp 0x14 bodies and {verified_10} \
             tagless 0x10 bodies satisfy the rule"
        );
        assert!(
            mismatches.is_empty(),
            "the rule misses:\n{}",
            mismatches.join("\n")
        );
        // The 0x14 form is the one a build holds; the 0x10 form is not claimed covered here.
        assert!(
            verified_14 > 0,
            "no zppp 0x14 clear PSO body was found to census"
        );
    }
}
